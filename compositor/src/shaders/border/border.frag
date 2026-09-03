// A rounded rectangular ring, drawn from nothing but its own geometry.
//
// Unlike the rounded-corner and blur shaders this is a *pixel* shader, not a
// texture one: a border has no buffer to sample, it generates every pixel it
// draws. Smithay therefore prepends `#version 100` itself, substitutes no
// `//_DEFINES_` line, and compiles only two variants — plain and DEBUG_FLAGS.

// Gives us fwidth() for scale-independent edge antialiasing. `enable` rather
// than `require` so drivers without it fall back to the fixed 1px width below.
#extension GL_OES_standard_derivatives : enable

// Two signed distance fields subtracted from each other needs more headroom
// than mediump gives us on a large output, so ask for highp where it exists.
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

// Additional uniforms, declared on the Rust side by `BorderShader`.
// Premultiplied, so scaling every channel by the mask keeps it that way.
uniform vec4 color;
// Ring width in physical pixels. Integral, so the outer edge lands exactly on
// the element boundary instead of half a pixel inside or outside it.
uniform float thickness;
// The *outer* radius: the window's own corner radius plus `thickness`, so the
// inner edge derived below traces the window's curve exactly.
uniform float radius;

// Signed distance to a rounded box, after Inigo Quilez. One radius rather than
// the rounded-corner shader's four: nothing rounds a border unevenly.
float rounded_box(in vec2 p, in vec2 b, in float r) {
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    vec2 half_size = size * 0.5;
    vec2 p = v_coords * size - half_size;

    // A radius wider than the element would fold the distance field inside out.
    float r_out = min(radius, min(half_size.x, half_size.y));
    float d_out = rounded_box(p, half_size, r_out);

    // The hole the window sits in. Clamped the same way, and to zero, because a
    // ring thicker than the window is a filled rectangle, not an inverted one.
    vec2 inner_half = max(half_size - vec2(thickness), vec2(0.0));
    float r_in = min(max(r_out - thickness, 0.0), min(inner_half.x, inner_half.y));
    float d_in = rounded_box(p, inner_half, r_in);

#if defined(GL_OES_standard_derivatives)
    // Width of one screen pixel in distance-field units, so both edges stay one
    // pixel wide however the element is scaled.
    float aa = max(fwidth(d_out), 0.0001);
#else
    float aa = 1.0;
#endif

    // Inside the outer edge and outside the inner one. Both antialiased, so a
    // hairline border fades rather than aliasing into a dashed line.
    float mask = (1.0 - smoothstep(-0.5 * aa, 0.5 * aa, d_out))
               * smoothstep(-0.5 * aa, 0.5 * aa, d_in);

    vec4 result = color * (alpha * mask);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        result = vec4(0.0, 0.2, 0.0, 0.2) + result * 0.8;
#endif

    gl_FragColor = result;
}
