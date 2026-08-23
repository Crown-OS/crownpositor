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

// Half a destination pixel in UV space, and the kawase spread multiplier.
uniform vec2 half_pixel;
uniform float offset;

// The taps reach past the texture edge by design; clamping half a pixel in
// keeps the edge from bleeding whatever the sampler's border color is.
vec2 cl(vec2 uv) {
    return clamp(uv, half_pixel, vec2(1.0) - half_pixel);
}

// Dual-kawase downsample: 5 taps, center weighted 4, corners 1 each.
// (Marius Bjørge, "Bandwidth-efficient rendering", SIGGRAPH 2015.)
void main() {
    vec2 o = half_pixel * offset;

    vec4 sum = texture2D(tex, cl(v_coords)) * 4.0;
    sum += texture2D(tex, cl(v_coords + vec2(-o.x, -o.y)));
    sum += texture2D(tex, cl(v_coords + vec2( o.x, -o.y)));
    sum += texture2D(tex, cl(v_coords + vec2(-o.x,  o.y)));
    sum += texture2D(tex, cl(v_coords + vec2( o.x,  o.y)));

    gl_FragColor = sum / 8.0;
}
