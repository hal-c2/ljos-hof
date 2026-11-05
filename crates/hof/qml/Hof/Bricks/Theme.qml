pragma Singleton
import QtQuick
import Hof.App

// The roles the bricks colour themselves by, grown from the terminal's own
// colours: the window's ground and text, and the theme's blue, red and
// yellow for the accent, errors and warnings. So the controls wear whatever
// theme the terminal does, light or dark.
QtObject {
    id: theme

    // The bricks ask `Theme.palette.color(role, fallback)`.
    readonly property QtObject palette: theme
    readonly property string appearance: luminance(Settings.background) < 0.18 ? "dark" : "light"
    readonly property real fontScale: 1
    readonly property real radius: 8
    readonly property bool frameless: false

    readonly property color ground: Settings.background
    readonly property color ink: Settings.foreground
    readonly property color accent: Settings.palette.length > 4 ? Settings.palette[4] : "#3b82f6"
    readonly property color danger: Settings.palette.length > 1 ? Settings.palette[1] : "#ef4444"
    readonly property color warning: Settings.palette.length > 3 ? Settings.palette[3] : "#eab308"

    // The surfaces, from the ground up: a card, a menu, the fill of a
    // row under the pointer.
    readonly property color raised: mix(ground, ink, 0.04)
    readonly property color overlay: mix(ground, ink, 0.06)
    readonly property color hover: mix(ground, ink, 0.11)
    // A control's edge stands out 3:1 from a card (WCAG's 1.4.11); a
    // field is filled with a third of it.
    readonly property color edge: reach(mix(ground, ink, 0.15), ink, raised, 3.1)
    readonly property color field: mix(raised, edge, 0.32)
    // Muted text is read on the most lifted of the surfaces, a field or a
    // row under the pointer, so it is made to pass there (4.5:1).
    readonly property color lifted: contrast(ink, field) < contrast(ink, hover) ? field : hover
    readonly property color muted: reach(mix(ink, ground, 0.5), ink, lifted, 4.6)

    readonly property var colors: ({
            "canvas": ground,
            "surface": ground,
            "surfaceRaised": raised,
            "surfaceOverlay": overlay,
            // A control's edge, as HAL-C2's palette has it.
            "input": edge,
            "border": mix(ground, ink, 0.12),
            "text": ink,
            "textMuted": muted,
            "secondaryLabel": muted,
            "placeholder": muted,
            "iconMuted": reach(mix(ink, ground, 0.5), ink, lifted, 3.1),
            "accent": accent,
            "accentForeground": contrast(accent, Qt.rgba(1, 1, 1, 1)) >= contrast(accent, Qt.rgba(0, 0, 0, 1)) ? "#ffffff" : "#000000",
            "accentSurface": hover,
            "focus": reach(accent, ink, raised, 3.1),
            "errorForeground": reach(danger, ink, raised, 4.6),
            "warningForeground": reach(warning, ink, raised, 4.6)
        })

    function color(role, fallback) {
        return colors[role] ?? fallback;
    }

    // `from` moved `amount` of the way to `to`, opaque.
    function mix(from, to, amount) {
        return Qt.rgba(from.r + (to.r - from.r) * amount, from.g + (to.g - from.g) * amount, from.b + (to.b - from.b) * amount, 1);
    }

    // WCAG's relative luminance.
    function luminance(c) {
        const linear = v => v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
        return 0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b);
    }

    // WCAG's contrast ratio of two colours, 1 to 21.
    function contrast(a, b) {
        const x = luminance(a);
        const y = luminance(b);
        return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
    }

    // The first colour on the way from `from` to `to` that stands out
    // `ratio` from `against`; `to` when none short of it does.
    function reach(from, to, against, ratio) {
        for (let t = 0; t < 1; t += 0.05) {
            const c = mix(from, to, t);
            if (contrast(c, against) >= ratio)
                return c;
        }
        return to;
    }
}
