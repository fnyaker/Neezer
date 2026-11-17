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
 * launcher itself can only ever show its own mask. This loads the APK's real
 * resources with `AssetManager.addAssetPath` and draws them through
 * `AdaptiveIconDrawable` — the class the launcher uses — once per AOSP mask,
 * plus the monochrome layer tinted in the two themed modes and the 24 dp
 * status-bar glyph. `render-masks.sh` drives it end to end.
 *
 * Outputs (PNG, 512×512 unless stated):
 *   mask-circle.png, mask-squircle.png, mask-teardrop.png   the icon, masked
 *   mono-dark.png, mono-light.png                           the themed icon
 *   glyph-24dp.png (63×63), glyph-24dp-x4.png               the status glyph
 */
public final class IconMasks {

    private static final int SIZE = 512;

    public static void main(String[] args) throws Exception {
        String apk = args[0], dir = args[1], pkg = args[2];

        AssetManager am = AssetManager.class.getConstructor().newInstance();
        // Hidden API, and fine here: app_process has no app and no targetSdk.
        AssetManager.class.getMethod("addAssetPath", String.class).invoke(am, apk);
        DisplayMetrics dm = new DisplayMetrics();
        dm.setToDefaults();
        Resources res = new Resources(am, dm, new Configuration());

        int bg = res.getColor(res.getIdentifier("ic_launcher_background", "color", pkg));
        Drawable fg = res.getDrawable(
                res.getIdentifier("ic_launcher_foreground", "drawable", pkg), null);
        int monoId = res.getIdentifier("ic_launcher_monochrome", "drawable", pkg);
        Drawable mono = monoId != 0 ? res.getDrawable(monoId, null) : null;

        // An adaptive icon's layers are 108 dp; a launcher shows the inner
        // 72 dp and then applies the mask. Same geometry as the launcher.
        float scale = SIZE / 72f;
        float off = -(108f * scale - SIZE) / 2f;

        for (String name : new String[]{"circle", "squircle", "teardrop"}) {
            Bitmap bmp = Bitmap.createBitmap(SIZE, SIZE, Bitmap.Config.ARGB_8888);
            Canvas c = new Canvas(bmp);
            c.save();
            c.clipPath(mask(name), android.graphics.Region.Op.REPLACE);
            c.drawColor(bg);
            c.save();
            c.translate(off, off);
            c.scale(scale, scale);
            fg.setBounds(0, 0, 108, 108);
            fg.draw(c);
            c.restore();
            c.restore();
            write(bmp, dir + "/mask-" + name + ".png");
        }

        if (mono == null) {
            System.err.println("no ic_launcher_monochrome in " + apk
                    + " (the themed icon cannot be shown)");
        } else {
            // As Android 13+ draws it: one tinted silhouette on the theme's
            // ground. These are the two modes that exist.
            themed(mono, dir + "/mono-dark.png", "#f3f0e8", "#1e1e1b", scale, off);
            themed(mono, dir + "/mono-light.png", "#0c0c0b", "#d6d1c3", scale, off);
        }

        int glyphId = res.getIdentifier("ic_stat_neezer", "drawable", pkg);
        if (glyphId != 0) {
            Drawable glyph = res.getDrawable(glyphId, null);
            // 24 dp at this screen's density (420 dpi), on a status-bar grey.
            int px = Math.round(24 * dm.density);
            Bitmap bmp = Bitmap.createBitmap(px, px, Bitmap.Config.ARGB_8888);
            Canvas c = new Canvas(bmp);
            c.drawColor(Color.parseColor("#1a1a1a"));
            glyph.setBounds(0, 0, px, px);
            glyph.setColorFilter(new PorterDuffColorFilter(Color.WHITE,
                    PorterDuff.Mode.SRC_IN));
            glyph.draw(c);
            write(bmp, dir + "/glyph-24dp.png");
            Bitmap big = Bitmap.createScaledBitmap(bmp, px * 4, px * 4, false);
            write(big, dir + "/glyph-24dp-x4.png");
        }
    }

    private static void themed(Drawable mono, String out, String tint, String ground,
                               float scale, float off) throws Exception {
        Bitmap bmp = Bitmap.createBitmap(SIZE, SIZE, Bitmap.Config.ARGB_8888);
        Canvas c = new Canvas(bmp);
        Paint p = new Paint(Paint.ANTI_ALIAS_FLAG);
        p.setColor(Color.parseColor(ground));
        c.drawCircle(SIZE / 2f, SIZE / 2f, SIZE / 2f, p);
        c.save();
        c.clipPath(mask("circle"), android.graphics.Region.Op.REPLACE);
        ColorFilter cf = new PorterDuffColorFilter(Color.parseColor(tint),
                                                   PorterDuff.Mode.SRC_IN);
        c.translate(off, off);
        c.scale(scale, scale);
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
