#version 100

// Replaced by the renderer with the `#define`s for the variant being compiled:
// EXTERNAL, NO_ALPHA and DEBUG_FLAGS. Must stay on a line of its own.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

// Gives us fwidth() for scale-independent edge antialiasing. `enable` rather
// than `require` so drivers without it fall back to the fixed 1px width below.
#extension GL_OES_standard_derivatives : enable

// The signed distance math needs more headroom than mediump gives us for large
// outputs, so ask for highp where the implementation supports it.
#if defined(GL_FRAGMENT_PRECISION_HIGH)
precision highp float;
#else
precision mediump float;
#endif

#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// Additional uniforms, declared on the Rust side by `RoundedCornerShader`.
// Element size in pixels, in the same space as `v_coords`.
uniform vec2 size;
// Corner radius in pixels.
uniform float radius;

// Signed distance to a box with per-corner radii, after Inigo Quilez.
// `r` holds the radii of the (+x, +y), (+x, -y), (-x, +y) and (-x, -y) corners.
float rounded_box(in vec2 p, in vec2 b, in vec4 r) {
    r.xy = (p.x > 0.0) ? r.xy : r.zw;
    r.x = (p.y > 0.0) ? r.x : r.y;
    vec2 q = abs(p) - b + r.x;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r.x;
}

void main() {
    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif

    vec2 half_size = size * 0.5;
    // A radius wider than the element would fold the distance field inside out.
    float r = min(radius, min(half_size.x, half_size.y));
    float distance = rounded_box(v_coords * size - half_size, half_size, vec4(r));

#if defined(GL_OES_standard_derivatives)
    // Width of one screen pixel in distance-field units, so the edge stays one
    // pixel wide however the element is scaled.
    float aa = max(fwidth(distance), 0.0001);
#else
    float aa = 1.0;
#endif
    float mask = 1.0 - smoothstep(-0.5 * aa, 0.5 * aa, distance);

    // The texture is premultiplied, so scaling every channel keeps it that way.
    color = color * (alpha * mask);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
