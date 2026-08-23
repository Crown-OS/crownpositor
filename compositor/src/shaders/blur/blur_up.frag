#version 100

// Replaced by the renderer with the `#define`s for the variant being compiled:
// EXTERNAL, NO_ALPHA and DEBUG_FLAGS. Must stay on a line of its own.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

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

// Half a *source* pixel in UV space, and the kawase spread multiplier.
uniform vec2 half_pixel;
uniform float offset;

vec2 cl(vec2 uv) {
    return clamp(uv, half_pixel, vec2(1.0) - half_pixel);
}

// Dual-kawase upsample: 8 taps, diagonals weighted 2, edges 1.
void main() {
    vec2 o = half_pixel * offset;

    vec4 sum = vec4(0.0);

    // Four edge centers.
    sum += texture2D(tex, cl(v_coords + vec2(-o.x * 2.0, 0.0)));
    sum += texture2D(tex, cl(v_coords + vec2( o.x * 2.0, 0.0)));
    sum += texture2D(tex, cl(v_coords + vec2(0.0, -o.y * 2.0)));
    sum += texture2D(tex, cl(v_coords + vec2(0.0,  o.y * 2.0)));

    // Four diagonals.
    sum += texture2D(tex, cl(v_coords + vec2(-o.x,  o.y))) * 2.0;
    sum += texture2D(tex, cl(v_coords + vec2( o.x,  o.y))) * 2.0;
    sum += texture2D(tex, cl(v_coords + vec2(-o.x, -o.y))) * 2.0;
    sum += texture2D(tex, cl(v_coords + vec2( o.x, -o.y))) * 2.0;

    gl_FragColor = sum / 12.0;
}
