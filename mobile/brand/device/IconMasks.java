import android.content.res.AssetManager;
import android.content.res.Configuration;
import android.content.res.Resources;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.ColorFilter;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.PorterDuff;
import android.graphics.PorterDuffColorFilter;
import android.graphics.drawable.Drawable;
import android.util.DisplayMetrics;

import java.io.FileOutputStream;

/**
 * Renders an installed APK's launcher icon and notification glyph the way the
 * platform does, on the device, with the framework's own drawables.
 *
 *   CLASSPATH=/data/local/tmp/lg1/classes.dex app_process /system/bin IconMasks \
 *       <base.apk> <out-dir> <resource-package>
 *
 * Why this exists: the lot's emulator image (API 35 "default") ships neither
 * the `com.android.theme.icon.*` overlays nor the themed-icons setting, so the
 * launcher itself can only show its own mask. This loads the APK's real
 * resources with `AssetManager.addAssetPath` twice — once with
 * UI_MODE_NIGHT_NO and once with UI_MODE_NIGHT_YES, which is how the launcher
 * picks `drawable/` against `drawable-night/` — and draws:
 *
 *   icon-day.png, icon-night.png           512×512, the composed icon
 *   mask-<shape>[-night].png               512×512, under the AOSP mask
 *   mono-dark.png, mono-light.png          the themed icon, both tint modes
 *   glyph-24dp.png, glyph-24dp-x4.png      the status glyph at this density
 *
 * The background is a DRAWABLE (`@drawable/ic_launcher_background`, with its
 * `drawable-night/` twin), not a colour: that is what makes the icon follow the
 * theme instead of pinning a black tile.
 */
public final class IconMasks {

    private static final int SIZE = 512;

    public static void main(String[] args) throws Exception {
        String apk = args[0], dir = args[1], pkg = args[2];

        // One AssetManager PER configuration: they share a single configuration
        // internally, so reusing one would make the second theme leak into the
        // first — measured, both renders came out night before this.
        Resources day = resFor(apk, Configuration.UI_MODE_NIGHT_NO);
        Resources night = resFor(apk, Configuration.UI_MODE_NIGHT_YES);

        render(day, dir, pkg, "day", false);
        render(night, dir, pkg, "night", true);

        // The monochrome layer is a mask: it resolves the same in both themes,
        // the launcher only tints it. Both tint modes are the ones that exist.
        Drawable mono = drawable(day, pkg, "drawable", "ic_launcher_monochrome");
        if (mono == null) {
            System.err.println("no ic_launcher_monochrome in " + apk);
        } else {
            themed(mono, dir + "/mono-dark.png", "#f3f0e8", "#1e1e1b");
            themed(mono, dir + "/mono-light.png", "#0c0c0b", "#d6d1c3");
        }

        Drawable glyph = drawable(day, pkg, "drawable", "ic_stat_neezer");
        if (glyph != null) {
            DisplayMetrics dm = new DisplayMetrics();
            dm.setToDefaults();
            int px = Math.round(24 * dm.density);
            Bitmap bmp = Bitmap.createBitmap(px, px, Bitmap.Config.ARGB_8888);
            Canvas c = new Canvas(bmp);
            c.drawColor(Color.parseColor("#1a1a1a"));
            glyph.setBounds(0, 0, px, px);
            glyph.setColorFilter(new PorterDuffColorFilter(Color.WHITE,
                    PorterDuff.Mode.SRC_IN));
            glyph.draw(c);
            write(bmp, dir + "/glyph-24dp.png");
            write(Bitmap.createScaledBitmap(bmp, px * 4, px * 4, false),
                    dir + "/glyph-24dp-x4.png");
        }
    }

    private static Resources resFor(String apk, int nightMode) throws Exception {
        AssetManager am = AssetManager.class.getConstructor().newInstance();
        // Hidden API, and fine here: app_process has no app and no targetSdk.
        AssetManager.class.getMethod("addAssetPath", String.class).invoke(am, apk);
        DisplayMetrics dm = new DisplayMetrics();
        dm.setToDefaults();
        Configuration c = new Configuration();
        c.uiMode = (c.uiMode & ~Configuration.UI_MODE_NIGHT_MASK) | nightMode;
        return new Resources(am, dm, c);
    }

    private static Drawable drawable(Resources res, String pkg, String type, String name) {
        int id = res.getIdentifier(name, type, pkg);
        return id != 0 ? res.getDrawable(id, null) : null;
    }

    private static void render(Resources res, String dir, String pkg, String theme,
                               boolean night) throws Exception {
        Drawable bg = drawable(res, pkg, "drawable", "ic_launcher_background");
        Drawable fg = drawable(res, pkg, "drawable", "ic_launcher_foreground");
        if (bg == null || fg == null) {
            System.err.println("missing background or foreground for " + theme);
            return;
        }
        // Layers are 108 dp; a launcher shows the inner 72 dp.
        float scale = SIZE / 72f;
        float off = -(108f * scale - SIZE) / 2f;

        // composed, unmasked
        Bitmap plain = Bitmap.createBitmap(SIZE, SIZE, Bitmap.Config.ARGB_8888);
        Canvas pc = new Canvas(plain);
        bg.setBounds(0, 0, SIZE, SIZE);
        bg.draw(pc);
        pc.save();
        pc.translate(off, off);
        pc.scale(scale, scale);
        fg.setBounds(0, 0, 108, 108);
        fg.draw(pc);
        pc.restore();
        write(plain, dir + "/icon-" + theme + ".png");

        // under each AOSP mask
        for (String name : new String[]{"circle", "squircle", "teardrop"}) {
            Bitmap bmp = Bitmap.createBitmap(SIZE, SIZE, Bitmap.Config.ARGB_8888);
            Canvas c = new Canvas(bmp);
            c.save();
            c.clipPath(mask(name), android.graphics.Region.Op.REPLACE);
            bg.setBounds(0, 0, SIZE, SIZE);
            bg.draw(c);
            c.save();
            c.translate(off, off);
            c.scale(scale, scale);
            fg.setBounds(0, 0, 108, 108);
            fg.draw(c);
            c.restore();
            c.restore();
            write(bmp, dir + "/mask-" + name + (night ? "-night" : "") + ".png");
        }
    }

    private static void themed(Drawable mono, String out, String tint, String ground)
            throws Exception {
        Bitmap bmp = Bitmap.createBitmap(SIZE, SIZE, Bitmap.Config.ARGB_8888);
        Canvas c = new Canvas(bmp);
        Paint p = new Paint(Paint.ANTI_ALIAS_FLAG);
        p.setColor(Color.parseColor(ground));
        c.drawCircle(SIZE / 2f, SIZE / 2f, SIZE / 2f, p);
        c.save();
        c.clipPath(mask("circle"), android.graphics.Region.Op.REPLACE);
        ColorFilter cf = new PorterDuffColorFilter(Color.parseColor(tint),
                                                   PorterDuff.Mode.SRC_IN);
        c.translate(-((108f * SIZE / 72f - SIZE) / 2f),
                    -((108f * SIZE / 72f - SIZE) / 2f));
        c.scale(SIZE / 72f, SIZE / 72f);
        mono.setBounds(0, 0, 108, 108);
        mono.setColorFilter(cf);
        mono.draw(c);
        c.restore();
        write(bmp, out);
    }

    /** The AOSP mask geometry, in a 0..512 box. */
    private static Path mask(String name) {
        Path p = new Path();
        switch (name) {
            case "circle":
                p.addCircle(SIZE / 2f, SIZE / 2f, SIZE / 2f, Path.Direction.CW);
                break;
            case "squircle": {
                // |x|^4 + |y|^4 = 1, sampled: the superellipse of a squircle.
                int n = 240;
                for (int i = 0; i <= n; i++) {
                    double t = 2 * Math.PI * i / n;
                    double cx = Math.cos(t), sy = Math.sin(t);
                    double x = Math.signum(cx) * Math.pow(Math.abs(cx), 0.5);
                    double y = Math.signum(sy) * Math.pow(Math.abs(sy), 0.5);
                    float px = (float) (SIZE / 2f + x * SIZE / 2f);
                    float py = (float) (SIZE / 2f + y * SIZE / 2f);
                    if (i == 0) {
                        p.moveTo(px, py);
                    } else {
                        p.lineTo(px, py);
                    }
                }
                p.close();
                break;
            }
            case "teardrop":
            default:
                // A circle with one sharp corner, at the top left.
                p.moveTo(0, 0);
                p.lineTo(SIZE / 2f, 0);
                p.arcTo(0, 0, SIZE, SIZE, -90, 270, false);
                p.close();
                break;
        }
        return p;
    }

    private static void write(Bitmap bmp, String path) throws Exception {
        try (FileOutputStream out = new FileOutputStream(path)) {
            bmp.compress(Bitmap.CompressFormat.PNG, 100, out);
        }
        System.out.println("wrote " + path);
    }
}
