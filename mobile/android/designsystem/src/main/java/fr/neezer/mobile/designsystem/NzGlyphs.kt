package fr.neezer.mobile.designsystem

// GENERATED from `webapp/src/components/Icon.svelte` — do not edit by hand.
//
// One `ImageVector` per glyph of the SPA's `Icon.svelte` (Lucide-style line
// icons, no emoji and no Material icon font: rule 5). The path data is the
// component's own `d`/`circle`/`rect`/`polygon` converted to SVG paths, which
// Compose parses with `addPathNodes` — so a glyph that changes in the SPA is
// regenerated here, not redrawn by hand.
//
// Every vector is drawn in BLACK and stroked at the SPA's 2 px with round caps:
// `Icon()` tints it through a `ColorFilter`, so the colour always comes from the
// theme (Sillon.colors), never from the file.

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

/** Every glyph the app may draw. */
enum class NzGlyph {
    Home,
    Bluetooth,
    Speaker,
    Search,
    SearchText,
    Library,
    Play,
    Pause,
    Prev,
    Next,
    Shuffle,
    Repeat,
    Repeat1,
    Heart,
    HeartFilled,
    More,
    MoreVertical,
    Queue,
    Chapters,
    Volume,
    Mute,
    Plus,
    Close,
    Leaf,
    Activity,
    Monitor,
    Cast,
    Info,
    Alert,
    ChevronDown,
    ChevronUp,
    Maximize,
    Minimize,
    Check,
    Youtube,
    CloudOff,
    ChevronLeft,
    ChevronRight,
    Radio,
    Infinity,
    Disc,
    User,
    Folder,
    FolderPlus,
    FolderMinus,
    FolderMove,
    Open,
    Music,
    Download,
    Upload,
    Sort,
    Sliders,
    Grip,
    Edit,
    Trash,
    MinusCircle,
    Refresh,
    Settings,
    Downloaded,
    Share,
    Bookmark,
    BookmarkFilled,
    BookmarkPlus,
    ZoomIn,
    ZoomOut,
    Scissors,
    Archive,
    Mic,
    Transcript,
    Tag,
    Zap,
    Sparkles,
    Pin,
    Party,
    Users,
    People,
    PersonAdd,
    Copy,
    Link,
    Minus,
    LogOut,
    Timer,
    Remote,
    Eye,
    Shield,
    Server,
    Moon,
    Compass,
    ThumbsUp,
    ThumbsDown,
    ThumbsDownFilled,
    Gauge,
}

private fun vector(name: String, build: ImageVector.Builder.() -> Unit): ImageVector =
    ImageVector.Builder(
        name = name,
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).apply(build).build()

private val HomeVector: ImageVector by lazy {
    vector("home") {
        addPath(pathData = addPathNodes("m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M9 22V12h6v10"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val BluetoothVector: ImageVector by lazy {
    vector("bluetooth") {
        addPath(pathData = addPathNodes("m7 7 10 10-5 5V2l5 5L7 17"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val SpeakerVector: ImageVector by lazy {
    vector("speaker") {
        addPath(pathData = addPathNodes("M6,2h12a2,2 0 0,1 2,2v16a2,2 0 0,1 -2,2h-12a2,2 0 0,1 -2,-2v-16a2,2 0 0,1 2,-2z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8,14a4,4 0 1,0 8,0a4,4 0 1,0 -8,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12 6h.01"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val SearchVector: ImageVector by lazy {
    vector("search") {
        addPath(pathData = addPathNodes("M3,11a8,8 0 1,0 16,0a8,8 0 1,0 -16,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m21 21-4.3-4.3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val SearchTextVector: ImageVector by lazy {
    vector("searchText") {
        addPath(pathData = addPathNodes("M21 6H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M10 12H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M10 18H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M14,15a3,3 0 1,0 6,0a3,3 0 1,0 -6,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m21 19-1.9-1.9"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val LibraryVector: ImageVector by lazy {
    vector("library") {
        addPath(pathData = addPathNodes("M21 15V6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M18.5 18a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12 12H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M16 6H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12 18H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val PlayVector: ImageVector by lazy {
    vector("play") {
        addPath(pathData = addPathNodes("M6,3L20,12L6,21z"), fill = SolidColor(Color.Black))
    }
}

private val PauseVector: ImageVector by lazy {
    vector("pause") {
        addPath(pathData = addPathNodes("M7,4h2a1,1 0 0,1 1,1v14a1,1 0 0,1 -1,1h-2a1,1 0 0,1 -1,-1v-14a1,1 0 0,1 1,-1z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M15,4h2a1,1 0 0,1 1,1v14a1,1 0 0,1 -1,1h-2a1,1 0 0,1 -1,-1v-14a1,1 0 0,1 1,-1z"), fill = SolidColor(Color.Black))
    }
}

private val PrevVector: ImageVector by lazy {
    vector("prev") {
        addPath(pathData = addPathNodes("M19 20 9 12l10-8z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M6,4h0a1,1 0 0,1 1,1v14a1,1 0 0,1 -1,1h0a1,1 0 0,1 -1,-1v-14a1,1 0 0,1 1,-1z"), fill = SolidColor(Color.Black))
    }
}

private val NextVector: ImageVector by lazy {
    vector("next") {
        addPath(pathData = addPathNodes("M5 4l10 8-10 8z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M18,4h0a1,1 0 0,1 1,1v14a1,1 0 0,1 -1,1h0a1,1 0 0,1 -1,-1v-14a1,1 0 0,1 1,-1z"), fill = SolidColor(Color.Black))
    }
}

private val ShuffleVector: ImageVector by lazy {
    vector("shuffle") {
        addPath(pathData = addPathNodes("M2 18h1.4c1.3 0 2.5-.6 3.3-1.7l6.1-8.6c.7-1.1 2-1.7 3.3-1.7H22"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m18 2 4 4-4 4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M2 6h1.9c1.5 0 2.9.9 3.6 2.2"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M22 18h-5.9c-1.3 0-2.6-.7-3.3-1.8l-.5-.8"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m18 14 4 4-4 4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val RepeatVector: ImageVector by lazy {
    vector("repeat") {
        addPath(pathData = addPathNodes("m17 2 4 4-4 4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M3 11v-1a4 4 0 0 1 4-4h14"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m7 22-4-4 4-4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M21 13v1a4 4 0 0 1-4 4H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val Repeat1Vector: ImageVector by lazy {
    vector("repeat1") {
        addPath(pathData = addPathNodes("m17 2 4 4-4 4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M3 11v-1a4 4 0 0 1 4-4h14"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m7 22-4-4 4-4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M21 13v1a4 4 0 0 1-4 4H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M11 10h1v4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val HeartVector: ImageVector by lazy {
    vector("heart") {
        addPath(pathData = addPathNodes("M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.29 1.51 4.04 3 5.5l7 7z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val HeartFilledVector: ImageVector by lazy {
    vector("heartFilled") {
        addPath(pathData = addPathNodes("M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.29 1.51 4.04 3 5.5l7 7z"), fill = SolidColor(Color.Black))
    }
}

private val MoreVector: ImageVector by lazy {
    vector("more") {
        addPath(pathData = addPathNodes("M3.4,12a1.6,1.6 0 1,0 3.2,0a1.6,1.6 0 1,0 -3.2,0z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M10.4,12a1.6,1.6 0 1,0 3.2,0a1.6,1.6 0 1,0 -3.2,0z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M17.4,12a1.6,1.6 0 1,0 3.2,0a1.6,1.6 0 1,0 -3.2,0z"), fill = SolidColor(Color.Black))
    }
}

private val MoreVerticalVector: ImageVector by lazy {
    vector("moreVertical") {
        addPath(pathData = addPathNodes("M10.4,5a1.6,1.6 0 1,0 3.2,0a1.6,1.6 0 1,0 -3.2,0z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M10.4,12a1.6,1.6 0 1,0 3.2,0a1.6,1.6 0 1,0 -3.2,0z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M10.4,19a1.6,1.6 0 1,0 3.2,0a1.6,1.6 0 1,0 -3.2,0z"), fill = SolidColor(Color.Black))
    }
}

private val QueueVector: ImageVector by lazy {
    vector("queue") {
        addPath(pathData = addPathNodes("M3,6L16,6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M3,12L16,12"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M3,18L12,18"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m16 14 5 4-5 4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ChaptersVector: ImageVector by lazy {
    vector("chapters") {
        addPath(pathData = addPathNodes("M12 12H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M16 6H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12 18H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m16 12 5 3-5 3v-6Z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val VolumeVector: ImageVector by lazy {
    vector("volume") {
        addPath(pathData = addPathNodes("M11,5L6,9L2,9L2,15L6,15L11,19z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M15.54 8.46a5 5 0 0 1 0 7.07"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M19.07 4.93a10 10 0 0 1 0 14.14"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val MuteVector: ImageVector by lazy {
    vector("mute") {
        addPath(pathData = addPathNodes("M11,5L6,9L2,9L2,15L6,15L11,19z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M22,9L16,15"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M16,9L22,15"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val PlusVector: ImageVector by lazy {
    vector("plus") {
        addPath(pathData = addPathNodes("M12,5L12,19"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M5,12L19,12"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val CloseVector: ImageVector by lazy {
    vector("close") {
        addPath(pathData = addPathNodes("M18,6L6,18"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M6,6L18,18"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val LeafVector: ImageVector by lazy {
    vector("leaf") {
        addPath(pathData = addPathNodes("M11 20A7 7 0 0 1 9.8 6.1C15.5 5 17 4.48 19 2c1 2 2 4.18 2 8 0 5.5-4.78 10-10 10Z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M2 21c0-3 1.85-5.36 5.08-6C9.5 14.52 12 13 13 12"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ActivityVector: ImageVector by lazy {
    vector("activity") {
        addPath(pathData = addPathNodes("M22 12h-4l-3 9L9 3l-3 9H2"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val MonitorVector: ImageVector by lazy {
    vector("monitor") {
        addPath(pathData = addPathNodes("M4,3h16a2,2 0 0,1 2,2v10a2,2 0 0,1 -2,2h-16a2,2 0 0,1 -2,-2v-10a2,2 0 0,1 2,-2z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8 21h8"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12 17v4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val CastVector: ImageVector by lazy {
    vector("cast") {
        addPath(pathData = addPathNodes("M2 16.1A5 5 0 0 1 5.9 20"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M2 12.05A9 9 0 0 1 9.95 20"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M2 8V6a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M2 20h.01"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val InfoVector: ImageVector by lazy {
    vector("info") {
        addPath(pathData = addPathNodes("M3,12a9,9 0 1,0 18,0a9,9 0 1,0 -18,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12,11L12,16"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12,8L12,8"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val AlertVector: ImageVector by lazy {
    vector("alert") {
        addPath(pathData = addPathNodes("M10.3 3.9 1.9 18a2 2 0 0 0 1.7 3h16.8a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12,9L12,13"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12,17L12,17"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ChevronDownVector: ImageVector by lazy {
    vector("chevronDown") {
        addPath(pathData = addPathNodes("M6,9L12,15L18,9"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ChevronUpVector: ImageVector by lazy {
    vector("chevronUp") {
        addPath(pathData = addPathNodes("M6,15L12,9L18,15"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val MaximizeVector: ImageVector by lazy {
    vector("maximize") {
        addPath(pathData = addPathNodes("M15 3h6v6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M9 21H3v-6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M21 3l-7 7"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M3 21l7-7"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val MinimizeVector: ImageVector by lazy {
    vector("minimize") {
        addPath(pathData = addPathNodes("M21 9h-6V3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M3 15h6v6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M14 10l7-7"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M10 14l-7 7"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val CheckVector: ImageVector by lazy {
    vector("check") {
        addPath(pathData = addPathNodes("M20,6L9,17L4,12"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val YoutubeVector: ImageVector by lazy {
    vector("youtube") {
        addPath(pathData = addPathNodes("M2.5 17a24.12 24.12 0 0 1 0-10 2 2 0 0 1 1.4-1.4 49.56 49.56 0 0 1 16.2 0A2 2 0 0 1 21.5 7a24.12 24.12 0 0 1 0 10 2 2 0 0 1-1.4 1.4 49.55 49.55 0 0 1-16.2 0A2 2 0 0 1 2.5 17"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m10 15 5-3-5-3z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val CloudOffVector: ImageVector by lazy {
    vector("cloudOff") {
        addPath(pathData = addPathNodes("m2 2 20 20"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M5.782 5.782A7 7 0 0 0 9 19h8.5a4.5 4.5 0 0 0 1.307-.193"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M21.532 16.5A4.5 4.5 0 0 0 17.5 10h-1.79A7.008 7.008 0 0 0 10 5.07"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ChevronLeftVector: ImageVector by lazy {
    vector("chevronLeft") {
        addPath(pathData = addPathNodes("M15,18L9,12L15,6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ChevronRightVector: ImageVector by lazy {
    vector("chevronRight") {
        addPath(pathData = addPathNodes("M9,6L15,12L9,18"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val RadioVector: ImageVector by lazy {
    vector("radio") {
        addPath(pathData = addPathNodes("M10,12a2,2 0 1,0 4,0a2,2 0 1,0 -4,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M4.93 19.07a10 10 0 0 1 0-14.14"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M7.76 16.24a6 6 0 0 1 0-8.48"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M16.24 7.76a6 6 0 0 1 0 8.48"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M19.07 4.93a10 10 0 0 1 0 14.14"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val InfinityVector: ImageVector by lazy {
    vector("infinity") {
        addPath(pathData = addPathNodes("M12 12c-2-2.67-4-4-6-4a4 4 0 1 0 0 8c2 0 4-1.33 6-4Zm0 0c2 2.67 4 4 6 4a4 4 0 0 0 0-8c-2 0-4 1.33-6 4Z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val DiscVector: ImageVector by lazy {
    vector("disc") {
        addPath(pathData = addPathNodes("M2,12a10,10 0 1,0 20,0a10,10 0 1,0 -20,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M10,12a2,2 0 1,0 4,0a2,2 0 1,0 -4,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val UserVector: ImageVector by lazy {
    vector("user") {
        addPath(pathData = addPathNodes("M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8,7a4,4 0 1,0 8,0a4,4 0 1,0 -8,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val FolderVector: ImageVector by lazy {
    vector("folder") {
        addPath(pathData = addPathNodes("M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val FolderPlusVector: ImageVector by lazy {
    vector("folderPlus") {
        addPath(pathData = addPathNodes("M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12 10v6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M9 13h6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val FolderMinusVector: ImageVector by lazy {
    vector("folderMinus") {
        addPath(pathData = addPathNodes("M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M9 13h6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val FolderMoveVector: ImageVector by lazy {
    vector("folderMove") {
        addPath(pathData = addPathNodes("M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8 13h7"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m12 10 3 3-3 3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val OpenVector: ImageVector by lazy {
    vector("open") {
        addPath(pathData = addPathNodes("M15 3h6v6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M10 14 21 3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val MusicVector: ImageVector by lazy {
    vector("music") {
        addPath(pathData = addPathNodes("M9 18V5l12-2v13"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M3,18a3,3 0 1,0 6,0a3,3 0 1,0 -6,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M15,16a3,3 0 1,0 6,0a3,3 0 1,0 -6,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val DownloadVector: ImageVector by lazy {
    vector("download") {
        addPath(pathData = addPathNodes("M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M7,10L12,15L17,10"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12,15L12,3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val UploadVector: ImageVector by lazy {
    vector("upload") {
        addPath(pathData = addPathNodes("M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M17,8L12,3L7,8"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12,3L12,15"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val SortVector: ImageVector by lazy {
    vector("sort") {
        addPath(pathData = addPathNodes("M11 5h10"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M11 9h7"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M11 13h4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m3 17 3 3 3-3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M6 18V4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val SlidersVector: ImageVector by lazy {
    vector("sliders") {
        addPath(pathData = addPathNodes("M4,21L4,14"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M4,10L4,3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12,21L12,12"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12,8L12,3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M20,21L20,16"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M20,12L20,3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M1,14L7,14"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M9,8L15,8"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M17,16L23,16"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val GripVector: ImageVector by lazy {
    vector("grip") {
        addPath(pathData = addPathNodes("M7.6,6a1.4,1.4 0 1,0 2.8,0a1.4,1.4 0 1,0 -2.8,0z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M7.6,12a1.4,1.4 0 1,0 2.8,0a1.4,1.4 0 1,0 -2.8,0z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M7.6,18a1.4,1.4 0 1,0 2.8,0a1.4,1.4 0 1,0 -2.8,0z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M13.6,6a1.4,1.4 0 1,0 2.8,0a1.4,1.4 0 1,0 -2.8,0z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M13.6,12a1.4,1.4 0 1,0 2.8,0a1.4,1.4 0 1,0 -2.8,0z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M13.6,18a1.4,1.4 0 1,0 2.8,0a1.4,1.4 0 1,0 -2.8,0z"), fill = SolidColor(Color.Black))
    }
}

private val EditVector: ImageVector by lazy {
    vector("edit") {
        addPath(pathData = addPathNodes("M12 20h9"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val TrashVector: ImageVector by lazy {
    vector("trash") {
        addPath(pathData = addPathNodes("M3 6h18"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M10,11L10,17"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M14,11L14,17"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val MinusCircleVector: ImageVector by lazy {
    vector("minusCircle") {
        addPath(pathData = addPathNodes("M2,12a10,10 0 1,0 20,0a10,10 0 1,0 -20,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8,12L16,12"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val RefreshVector: ImageVector by lazy {
    vector("refresh") {
        addPath(pathData = addPathNodes("M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M21 3v5h-5"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M3 21v-5h5"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val SettingsVector: ImageVector by lazy {
    vector("settings") {
        addPath(pathData = addPathNodes("M9,12a3,3 0 1,0 6,0a3,3 0 1,0 -6,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val DownloadedVector: ImageVector by lazy {
    vector("downloaded") {
        addPath(pathData = addPathNodes("M2,12a10,10 0 1,0 20,0a10,10 0 1,0 -20,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8,12L11,15L16,9"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ShareVector: ImageVector by lazy {
    vector("share") {
        addPath(pathData = addPathNodes("M15,5a3,3 0 1,0 6,0a3,3 0 1,0 -6,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M3,12a3,3 0 1,0 6,0a3,3 0 1,0 -6,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M15,19a3,3 0 1,0 6,0a3,3 0 1,0 -6,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8.59,13.51L15.42,17.49"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M15.41,6.51L8.59,10.49"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val BookmarkVector: ImageVector by lazy {
    vector("bookmark") {
        addPath(pathData = addPathNodes("m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val BookmarkFilledVector: ImageVector by lazy {
    vector("bookmarkFilled") {
        addPath(pathData = addPathNodes("m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"), fill = SolidColor(Color.Black))
    }
}

private val BookmarkPlusVector: ImageVector by lazy {
    vector("bookmarkPlus") {
        addPath(pathData = addPathNodes("m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12,7L12,13"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M9,10L15,10"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ZoomInVector: ImageVector by lazy {
    vector("zoomIn") {
        addPath(pathData = addPathNodes("M3,11a8,8 0 1,0 16,0a8,8 0 1,0 -16,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M21,21L16.65,16.65"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M11,8L11,14"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8,11L14,11"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ZoomOutVector: ImageVector by lazy {
    vector("zoomOut") {
        addPath(pathData = addPathNodes("M3,11a8,8 0 1,0 16,0a8,8 0 1,0 -16,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M21,21L16.65,16.65"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8,11L14,11"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ScissorsVector: ImageVector by lazy {
    vector("scissors") {
        addPath(pathData = addPathNodes("M3,6a3,3 0 1,0 6,0a3,3 0 1,0 -6,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8.12 8.12 12 12"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M20 4 8.12 15.88"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M3,18a3,3 0 1,0 6,0a3,3 0 1,0 -6,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M14.8 14.8 20 20"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ArchiveVector: ImageVector by lazy {
    vector("archive") {
        addPath(pathData = addPathNodes("M3,3h18a1,1 0 0,1 1,1v3a1,1 0 0,1 -1,1h-18a1,1 0 0,1 -1,-1v-3a1,1 0 0,1 1,-1z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M10 12h4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val MicVector: ImageVector by lazy {
    vector("mic") {
        addPath(pathData = addPathNodes("M12,2h0a3,3 0 0,1 3,3v6a3,3 0 0,1 -3,3h0a3,3 0 0,1 -3,-3v-6a3,3 0 0,1 3,-3z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M5 10a7 7 0 0 0 14 0"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12,17L12,21"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M8,21L16,21"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val TranscriptVector: ImageVector by lazy {
    vector("transcript") {
        addPath(pathData = addPathNodes("M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M13 8H7"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M17 12H7"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val TagVector: ImageVector by lazy {
    vector("tag") {
        addPath(pathData = addPathNodes("M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M6.5,7.5a1,1 0 1,0 2,0a1,1 0 1,0 -2,0z"), fill = SolidColor(Color.Black))
    }
}

private val ZapVector: ImageVector by lazy {
    vector("zap") {
        addPath(pathData = addPathNodes("M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val SparklesVector: ImageVector by lazy {
    vector("sparkles") {
        addPath(pathData = addPathNodes("M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M20 3v4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M22 5h-4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M4 17v2"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M5 18H3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val PinVector: ImageVector by lazy {
    vector("pin") {
        addPath(pathData = addPathNodes("M12 17v5"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val PartyVector: ImageVector by lazy {
    vector("party") {
        addPath(pathData = addPathNodes("M3 17v-3a9 9 0 0 1 18 0v3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M4,15h1a2,2 0 0,1 2,2v3a2,2 0 0,1 -2,2h-1a2,2 0 0,1 -2,-2v-3a2,2 0 0,1 2,-2z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M19,15h1a2,2 0 0,1 2,2v3a2,2 0 0,1 -2,2h-1a2,2 0 0,1 -2,-2v-3a2,2 0 0,1 2,-2z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M9.5 9.5a3.5 3.5 0 0 1 5 0"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M7.4 7.4a6.5 6.5 0 0 1 9.2 0"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val UsersVector: ImageVector by lazy {
    vector("users") {
        addPath(pathData = addPathNodes("M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M5,7a4,4 0 1,0 8,0a4,4 0 1,0 -8,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M22 21v-2a4 4 0 0 0-3-3.87"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M16 3.13a4 4 0 0 1 0 7.75"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val PeopleVector: ImageVector by lazy {
    vector("people") {
        addPath(pathData = addPathNodes("M18 21a8 8 0 0 0-16 0"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M5,8a5,5 0 1,0 10,0a5,5 0 1,0 -10,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M22 20c0-3.37-2-6.5-4-8a5 5 0 0 0-.45-8.3"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val PersonAddVector: ImageVector by lazy {
    vector("personAdd") {
        addPath(pathData = addPathNodes("M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M5,7a4,4 0 1,0 8,0a4,4 0 1,0 -8,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M19,8L19,14"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M22,11L16,11"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val CopyVector: ImageVector by lazy {
    vector("copy") {
        addPath(pathData = addPathNodes("M10,8h10a2,2 0 0,1 2,2v10a2,2 0 0,1 -2,2h-10a2,2 0 0,1 -2,-2v-10a2,2 0 0,1 2,-2z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val LinkVector: ImageVector by lazy {
    vector("link") {
        addPath(pathData = addPathNodes("M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val MinusVector: ImageVector by lazy {
    vector("minus") {
        addPath(pathData = addPathNodes("M5 12h14"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val LogOutVector: ImageVector by lazy {
    vector("logOut") {
        addPath(pathData = addPathNodes("M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m16 17 5-5-5-5"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M21 12H9"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val TimerVector: ImageVector by lazy {
    vector("timer") {
        addPath(pathData = addPathNodes("M10,2L14,2"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M12,14L15,11"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M4,14a8,8 0 1,0 16,0a8,8 0 1,0 -16,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val RemoteVector: ImageVector by lazy {
    vector("remote") {
        addPath(pathData = addPathNodes("M10,2h4a3,3 0 0,1 3,3v14a3,3 0 0,1 -3,3h-4a3,3 0 0,1 -3,-3v-14a3,3 0 0,1 3,-3z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M10,8a2,2 0 1,0 4,0a2,2 0 1,0 -4,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M10 13h.01M14 13h.01M10 16.5h.01M14 16.5h.01"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val EyeVector: ImageVector by lazy {
    vector("eye") {
        addPath(pathData = addPathNodes("M2.06 12.35a1 1 0 0 1 0-.7 10.75 10.75 0 0 1 19.88 0 1 1 0 0 1 0 .7 10.75 10.75 0 0 1-19.88 0"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M9,12a3,3 0 1,0 6,0a3,3 0 1,0 -6,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ShieldVector: ImageVector by lazy {
    vector("shield") {
        addPath(pathData = addPathNodes("M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ServerVector: ImageVector by lazy {
    vector("server") {
        addPath(pathData = addPathNodes("M4,2h16a2,2 0 0,1 2,2v4a2,2 0 0,1 -2,2h-16a2,2 0 0,1 -2,-2v-4a2,2 0 0,1 2,-2z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M4,14h16a2,2 0 0,1 2,2v4a2,2 0 0,1 -2,2h-16a2,2 0 0,1 -2,-2v-4a2,2 0 0,1 2,-2z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M6 6h.01M6 18h.01"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val MoonVector: ImageVector by lazy {
    vector("moon") {
        addPath(pathData = addPathNodes("M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val CompassVector: ImageVector by lazy {
    vector("compass") {
        addPath(pathData = addPathNodes("M2,12a10,10 0 1,0 20,0a10,10 0 1,0 -20,0z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("m16.24 7.76-2.12 6.36-6.36 2.12 2.12-6.36 6.36-2.12z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ThumbsUpVector: ImageVector by lazy {
    vector("thumbsUp") {
        addPath(pathData = addPathNodes("M7 10v12"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M15 5.88 14 10h5.83a2 2 0 0 1 1.92 2.56l-2.33 8A2 2 0 0 1 17.5 22H4a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h2.76a2 2 0 0 0 1.79-1.11L12 2a3.13 3.13 0 0 1 3 3.88Z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ThumbsDownVector: ImageVector by lazy {
    vector("thumbsDown") {
        addPath(pathData = addPathNodes("M17 14V2"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M9 18.12 10 14H4.17a2 2 0 0 1-1.92-2.56l2.33-8A2 2 0 0 1 6.5 2H20a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-2.76a2 2 0 0 0-1.79 1.11L12 22a3.13 3.13 0 0 1-3-3.88Z"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

private val ThumbsDownFilledVector: ImageVector by lazy {
    vector("thumbsDownFilled") {
        addPath(pathData = addPathNodes("M9 18.12 10 14H4.17a2 2 0 0 1-1.92-2.56l2.33-8A2 2 0 0 1 6.5 2H17v12h-1.76a2 2 0 0 0-1.79 1.11L12 22a3.13 3.13 0 0 1-3-3.88Z"), fill = SolidColor(Color.Black))
        addPath(pathData = addPathNodes("M17 14V2h3a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2z"), fill = SolidColor(Color.Black))
    }
}

private val GaugeVector: ImageVector by lazy {
    vector("gauge") {
        addPath(pathData = addPathNodes("m12 14 4-4"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        addPath(pathData = addPathNodes("M3.34 19a10 10 0 1 1 17.32 0"), fill = null, stroke = SolidColor(Color.Black), strokeLineWidth = 2f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
    }
}

/** The vector of one glyph, built once per process. */
fun nzVector(glyph: NzGlyph): ImageVector = when (glyph) {
    NzGlyph.Home -> HomeVector
    NzGlyph.Bluetooth -> BluetoothVector
    NzGlyph.Speaker -> SpeakerVector
    NzGlyph.Search -> SearchVector
    NzGlyph.SearchText -> SearchTextVector
    NzGlyph.Library -> LibraryVector
    NzGlyph.Play -> PlayVector
    NzGlyph.Pause -> PauseVector
    NzGlyph.Prev -> PrevVector
    NzGlyph.Next -> NextVector
    NzGlyph.Shuffle -> ShuffleVector
    NzGlyph.Repeat -> RepeatVector
    NzGlyph.Repeat1 -> Repeat1Vector
    NzGlyph.Heart -> HeartVector
    NzGlyph.HeartFilled -> HeartFilledVector
    NzGlyph.More -> MoreVector
    NzGlyph.MoreVertical -> MoreVerticalVector
    NzGlyph.Queue -> QueueVector
    NzGlyph.Chapters -> ChaptersVector
    NzGlyph.Volume -> VolumeVector
    NzGlyph.Mute -> MuteVector
    NzGlyph.Plus -> PlusVector
    NzGlyph.Close -> CloseVector
    NzGlyph.Leaf -> LeafVector
    NzGlyph.Activity -> ActivityVector
    NzGlyph.Monitor -> MonitorVector
    NzGlyph.Cast -> CastVector
    NzGlyph.Info -> InfoVector
    NzGlyph.Alert -> AlertVector
    NzGlyph.ChevronDown -> ChevronDownVector
    NzGlyph.ChevronUp -> ChevronUpVector
    NzGlyph.Maximize -> MaximizeVector
    NzGlyph.Minimize -> MinimizeVector
    NzGlyph.Check -> CheckVector
    NzGlyph.Youtube -> YoutubeVector
    NzGlyph.CloudOff -> CloudOffVector
    NzGlyph.ChevronLeft -> ChevronLeftVector
    NzGlyph.ChevronRight -> ChevronRightVector
    NzGlyph.Radio -> RadioVector
    NzGlyph.Infinity -> InfinityVector
    NzGlyph.Disc -> DiscVector
    NzGlyph.User -> UserVector
    NzGlyph.Folder -> FolderVector
    NzGlyph.FolderPlus -> FolderPlusVector
    NzGlyph.FolderMinus -> FolderMinusVector
    NzGlyph.FolderMove -> FolderMoveVector
    NzGlyph.Open -> OpenVector
    NzGlyph.Music -> MusicVector
    NzGlyph.Download -> DownloadVector
    NzGlyph.Upload -> UploadVector
    NzGlyph.Sort -> SortVector
    NzGlyph.Sliders -> SlidersVector
    NzGlyph.Grip -> GripVector
    NzGlyph.Edit -> EditVector
    NzGlyph.Trash -> TrashVector
    NzGlyph.MinusCircle -> MinusCircleVector
    NzGlyph.Refresh -> RefreshVector
    NzGlyph.Settings -> SettingsVector
    NzGlyph.Downloaded -> DownloadedVector
    NzGlyph.Share -> ShareVector
    NzGlyph.Bookmark -> BookmarkVector
    NzGlyph.BookmarkFilled -> BookmarkFilledVector
    NzGlyph.BookmarkPlus -> BookmarkPlusVector
    NzGlyph.ZoomIn -> ZoomInVector
    NzGlyph.ZoomOut -> ZoomOutVector
    NzGlyph.Scissors -> ScissorsVector
    NzGlyph.Archive -> ArchiveVector
    NzGlyph.Mic -> MicVector
    NzGlyph.Transcript -> TranscriptVector
    NzGlyph.Tag -> TagVector
    NzGlyph.Zap -> ZapVector
    NzGlyph.Sparkles -> SparklesVector
    NzGlyph.Pin -> PinVector
    NzGlyph.Party -> PartyVector
    NzGlyph.Users -> UsersVector
    NzGlyph.People -> PeopleVector
    NzGlyph.PersonAdd -> PersonAddVector
    NzGlyph.Copy -> CopyVector
    NzGlyph.Link -> LinkVector
    NzGlyph.Minus -> MinusVector
    NzGlyph.LogOut -> LogOutVector
    NzGlyph.Timer -> TimerVector
    NzGlyph.Remote -> RemoteVector
    NzGlyph.Eye -> EyeVector
    NzGlyph.Shield -> ShieldVector
    NzGlyph.Server -> ServerVector
    NzGlyph.Moon -> MoonVector
    NzGlyph.Compass -> CompassVector
    NzGlyph.ThumbsUp -> ThumbsUpVector
    NzGlyph.ThumbsDown -> ThumbsDownVector
    NzGlyph.ThumbsDownFilled -> ThumbsDownFilledVector
    NzGlyph.Gauge -> GaugeVector
}
