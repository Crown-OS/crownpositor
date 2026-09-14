#version 100

// Replaced by the renderer with the `#define`s for the variant being compiled:
// EXTERNAL, NO_ALPHA and DEBUG_FLAGS. Must stay on a line of its own.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

// fwidth() for scale-independent edge antialiasing; `enable` so drivers
// without it fall back to the fixed 1px band below.
#extension GL_OES_standard_derivatives : enable

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

// Where this backdrop sits in the framebuffer, in pixels. Every position below
// comes from `gl_FragCoord`, so a rotated or flipped output needs no special
// case: the framebuffer is the one space all of them agree on.
uniform vec2 backdrop_origin;
uniform vec2 backdrop_size;
// The rectangle the corners are cut from, in those same framebuffer pixels: the
// whole window, not the piece being drawn. A committed blur region is not
// always the whole surface, so one window's backdrop can be several rectangles,
// and each has to be cut by the window's corners rather than its own.
uniform vec2 mask_origin;
uniform vec2 mask_size;
// Half a pixel of the pyramid's top level in UV space, and the kawase spread:
// this pass *is* the last upsample, straight into the frame.
uniform vec2 half_pixel;
uniform float offset;
// Corner radius in pixels — must match the window drawn on top.
uniform float corner_radius;
// Dither strength; hides the banding a strong blur produces on gradients.
uniform float noise;

vec2 cl(vec2 uv) {
    return clamp(uv, half_pixel, vec2(1.0) - half_pixel);
}

// Signed distance to a rounded box, after Inigo Quilez.
float rounded_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

// Cheap screen-space hash (Dave Hoskins) for the dither.
float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

void main() {
    vec2 uv = (gl_FragCoord.xy - backdrop_origin) / backdrop_size;
    vec2 o = half_pixel * offset;

    // Dual-kawase upsample: 8 taps, diagonals weighted 2, edges 1.
    vec4 sum = vec4(0.0);
    sum += texture2D(tex, cl(uv + vec2(-o.x * 2.0, 0.0)));
    sum += texture2D(tex, cl(uv + vec2( o.x * 2.0, 0.0)));
    sum += texture2D(tex, cl(uv + vec2(0.0, -o.y * 2.0)));
    sum += texture2D(tex, cl(uv + vec2(0.0,  o.y * 2.0)));
    sum += texture2D(tex, cl(uv + vec2(-o.x,  o.y))) * 2.0;
    sum += texture2D(tex, cl(uv + vec2( o.x,  o.y))) * 2.0;
    sum += texture2D(tex, cl(uv + vec2(-o.x, -o.y))) * 2.0;
    sum += texture2D(tex, cl(uv + vec2( o.x, -o.y))) * 2.0;

    // What was copied out of the framebuffer was opaque; make that explicit so
    // stale alpha from the pyramid can never punch a hole in the backdrop.
    vec4 color = vec4((sum / 12.0).rgb, 1.0);

    // Everything below is measured in the masked rectangle's own space, so that
    // a backdrop split into pieces dithers and rounds as one surface.
    vec2 p = gl_FragCoord.xy - mask_origin;

    if (noise > 0.0) {
        float dither = (hash(p) - 0.5) * noise;
        color.rgb += vec3(dither);
    }

    vec2 half_size = mask_size * 0.5;
    float r = min(corner_radius, min(half_size.x, half_size.y));
    float distance = rounded_box(p - half_size, half_size, r);

#if defined(GL_OES_standard_derivatives)
    float aa = max(fwidth(distance), 0.0001);
#else
    float aa = 1.0;
#endif
    float mask = 1.0 - smoothstep(-0.5 * aa, 0.5 * aa, distance);

    // Premultiplied output: scale every channel.
    color = color * (alpha * mask);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
