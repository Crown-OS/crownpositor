// A window's titlebar: the tinted panel, its top highlight, and the three
// window controls — all generated from geometry in one pass.
//
// A *pixel* shader like border.frag, so smithay prepends `#version 100`,
// substitutes no `//_DEFINES_` line and compiles only the plain and
// DEBUG_FLAGS variants. There is no buffer to sample: the blurred glass behind
// this is a separate element drawn underneath, and everything here is tint
// laid over it.
//
// The controls live in the same pass as the panel because they are three
// circles and three glyphs — a handful of distance-field evaluations, far
// cheaper than three more elements for the damage tracker to track and three
// more draw calls to issue.

#extension GL_OES_standard_derivatives : enable

// Several distance fields subtracted from each other needs more headroom than
// mediump gives on a large output.
#if defined(GL_FRAGMENT_PRECISION_HIGH)
precision highp float;
#else
precision mediump float;
#endif

// Supplied by the renderer for every pixel program.
uniform vec2 size;
uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// The sheet's rect, relative to this element's own origin, and its corner
// radius. Everything is masked by this rounded rectangle, so the tint traces
// the window's curve exactly wherever the client does not cover it — the
// titlebar, the notches its rounded corners cut away, and the border ring.
uniform vec2 window_offset;
uniform vec2 window_size;
uniform float radius;

// Height of the titlebar band, measured down from the sheet's top edge. The
// gradient runs over this rather than over the whole element: the sheet is as
// tall as the window, and a gradient stretched over all of it would leave the
// titlebar a flat wash.
uniform float bar_height;

// Premultiplied gradient endpoints, top to bottom.
uniform vec4 tint_top;
uniform vec4 tint_bottom;
// Premultiplied hairline along the inside of the top edge.
uniform vec4 highlight;

// Centre of the first control, its spacing and radius, in physical pixels.
// Controls run left to right: maximize, minimize, close.
uniform vec2 control_origin;
uniform float control_spacing;
uniform float control_radius;
// Premultiplied disc and glyph colours.
uniform vec4 control_fill;
uniform vec4 control_glyph;
// Which control the pointer is over, as an index; anything outside 0..2 means
// none. Held as a float because that is what a `_1f` uniform carries.
uniform float control_hover;

float rounded_box(in vec2 p, in vec2 b, in float r) {
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

// Distance to a line segment, for the glyph strokes.
float segment(in vec2 p, in vec2 a, in vec2 b) {
    vec2 pa = p - a;
    vec2 ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// Coverage of a distance field's interior, antialiased over one screen pixel.
float coverage(in float distance, in float aa) {
    return 1.0 - smoothstep(-0.5 * aa, 0.5 * aa, distance);
}

// `over` compositing for premultiplied colours.
vec4 over(in vec4 dst, in vec4 src) {
    return src + dst * (1.0 - src.a);
}

// The three glyphs, drawn into a box of half-extent `h` centred on the origin.
// Distance, not coverage, so the caller antialiases them all the same way.
float glyph(in vec2 p, in float index, in float h, in float stroke) {
    if (index < 0.5) {
        // Maximize: a square outline.
        float outer = rounded_box(p, vec2(h), stroke * 0.5);
        float inner = rounded_box(p, vec2(h - stroke), 0.0);
        return max(outer, -inner);
    }
    if (index < 1.5) {
        // Minimize: a single bar.
        return segment(p, vec2(-h, 0.0), vec2(h, 0.0)) - stroke * 0.5;
    }
    // Close: two crossed strokes.
    float a = segment(p, vec2(-h, -h), vec2(h, h));
    float b = segment(p, vec2(-h, h), vec2(h, -h));
    return min(a, b) - stroke * 0.5;
}

void main() {
    vec2 p = v_coords * size;

#if defined(GL_OES_standard_derivatives)
    float aa = max(fwidth(p.x), 0.0001);
#else
    float aa = 1.0;
#endif

    // Masked by the window, not by the titlebar: the two have to agree at the
    // corners or a seam shows where they meet.
    vec2 window_half = window_size * 0.5;
    float r = min(radius, min(window_half.x, window_half.y));
    float mask = coverage(rounded_box(p - window_offset - window_half, window_half, r), aa);

    // The panel itself. Clamped, so everything below the bar carries the tint
    // the bar ended on and the two cannot read as different materials.
    float down = (p.y - window_offset.y) / max(bar_height, 1.0);
    vec4 color = mix(tint_top, tint_bottom, clamp(down, 0.0, 1.0));

    // A hairline just inside the top edge, faded out where the corners curve
    // away so it does not stop dead against the rounding.
    float from_top = p.y - window_offset.y;
    color = over(color, highlight * coverage(from_top - 1.0, aa));

    // The controls. Each is a disc with a glyph knocked into it; the hovered
    // one lifts rather than changing hue, which reads at any accent colour.
    for (int index = 0; index < 3; index++) {
        vec2 centre = control_origin + vec2(control_spacing * float(index), 0.0);
        vec2 local = p - centre;

        float lift = abs(float(index) - control_hover) < 0.5 ? 1.35 : 1.0;
        float disc = coverage(length(local) - control_radius, aa);
        color = over(color, control_fill * (disc * lift));

        float extent = control_radius * 0.42;
        float stroke = max(control_radius * 0.18, 1.0);
        float ink = coverage(glyph(local, float(index), extent, stroke), aa);
        color = over(color, control_glyph * (ink * disc));
    }

    vec4 result = color * (alpha * mask);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        result = vec4(0.0, 0.2, 0.0, 0.2) + result * 0.8;
#endif

    gl_FragColor = result;
}
