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

// The blur radius in texture UV units, per axis.
uniform vec2 reach;

const int TAPS = 48;
const float GOLDEN_ANGLE = 2.39996323;

// Taps on a golden-angle spiral fill the disc evenly at any radius, so one
// pass blurs smoothly enough for a layer that is moving while it blurs. The
// sprite is padded with transparent texels, so taps past its content fade
// rather than clamp.
void main() {
    vec4 sum = vec4(0.0);
    for (int i = 0; i < TAPS; i++) {
        float t = (float(i) + 0.5) / float(TAPS);
        float angle = float(i) * GOLDEN_ANGLE;
        vec2 offset = vec2(cos(angle), sin(angle)) * sqrt(t) * reach;
        sum += texture2D(tex, v_coords + offset);
    }
    vec4 color = sum / float(TAPS);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif

    color = color * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
