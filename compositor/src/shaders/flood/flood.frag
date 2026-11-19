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

// The output's top-left corner in framebuffer pixels, and the rows taking a
// framebuffer offset from it to the output's own 0..1 across and down.
uniform vec2 origin;
uniform vec2 across;
uniform vec2 down;
// Where the water wells up, in the output's own 0..1, and the output's width
// over its height.
uniform vec2 source;
uniform float aspect;
// How far it has spread: 0 before it wells up, 1 once its front is past the
// farthest corner.
uniform float level;

// Output heights past the farthest corner the front lands.
const float MARGIN = 0.05;
// Output heights the front fades in over.
const float SOFTNESS = 0.006;
// How far the lobes bulge out and pull in, as a fraction of the radius.
const float LOBES = 0.45;
// How far it has spread before the lobes start to round off, so the front
// is a blob for most of its way and still lands past every corner.
const float ROUNDING = 0.55;
// Radians the lobes turn by as the water spreads, so the blob keeps morphing.
const float DRIFT = 3.0;
// Output heights inside the front the water lenses and catches the light.
const float RIM = 0.03;
// Texture UV the rim bends what it carries by.
const float REFRACTION = 0.012;
const float SHEEN = 0.14;

float lobes(float angle, float drift) {
    return 0.45 * sin(3.0 * angle + 0.9 + drift)
        + 0.25 * sin(2.0 * angle + 4.1 + 0.6 * drift)
        + 0.18 * sin(5.0 * angle + 2.3 - 1.7 * drift)
        + 0.12 * sin(7.0 * angle + 1.1 + 2.3 * drift);
}

void main() {
    vec2 offset = gl_FragCoord.xy - origin;
    vec2 at = vec2(dot(across, offset), dot(down, offset));
    vec2 square = vec2(aspect, 1.0);
    vec2 outward = (at - source) * square;
    float far = length(outward);
    float reach = length(max(source, 1.0 - source) * square) + MARGIN;
    float unsettled = clamp(1.0 - level, 0.0, 1.0);
    float angle = atan(outward.y, outward.x + 1e-6);
    float radius = max(level, 0.0) * reach
        * (1.0 + LOBES * (1.0 - smoothstep(ROUNDING, 1.0, level)) * lobes(angle, DRIFT * level));
    float edge = radius - far;
    float cover = smoothstep(0.0, min(SOFTNESS, 0.5 * radius) + 1e-4, edge);

    float into_rim = edge / RIM - 0.5;
    float rim = exp(-4.0 * into_rim * into_rim) * unsettled;
    vec2 bend = REFRACTION * rim * outward / (square * (far + 1e-4));
    vec4 color = texture2D(tex, v_coords - bend);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif

    color.rgb = min(color.rgb + SHEEN * rim * color.a, vec3(color.a));
    color = color * (alpha * cover);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
