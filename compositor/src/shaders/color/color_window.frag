#version 100

// Replaced by the renderer with the `#define`s for the variant being compiled:
// EXTERNAL, NO_ALPHA and DEBUG_FLAGS. Must stay on a line of its own.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

#extension GL_OES_standard_derivatives : enable

// Colour work needs the headroom far more than the distance field does: PQ's
// inverse is a pair of chained `pow`s over four decades of luminance, and
// mediump would quantise the result visibly in shadows.
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

uniform vec2 size;
uniform vec4 radius;

// Which curve to decode with. Must match `color::pipeline::TransferKind`.
uniform int tf_kind;
// The power curve's exponent, or HLG's peak luminance in cd/m².
uniform float tf_param;
// Brings decoded light onto "1.0 is the output's reference white".
uniform float luminance_scale;
// Source primaries into the compositing space, column-major.
uniform mat3 primaries;

#define TF_LINEAR         0
#define TF_POWER          1
#define TF_SRGB_PIECEWISE 2
#define TF_SRGB_EXTENDED  3
#define TF_ST240          4
#define TF_ST2084_PQ      5
#define TF_HLG            6
#define TF_ST428          7

float rounded_box(in vec2 p, in vec2 b, in vec4 r);

// A power curve mirrored through the origin, so a wide-gamut encoding's
// negative channels stay negative instead of becoming NaN.
vec3 signed_pow(vec3 v, float e) {
    return sign(v) * pow(abs(v), vec3(e));
}

// IEC 61966-2-1.
vec3 srgb_to_linear(vec3 v) {
    return mix(
        v / 12.92,
        pow((v + 0.055) / 1.055, vec3(2.4)),
        step(vec3(0.04045), v)
    );
}

// SMPTE ST 2084, to absolute cd/m².
vec3 pq_to_nits(vec3 v) {
    const float m1 = 0.1593017578125;
    const float m2 = 78.84375;
    const float c1 = 0.8359375;
    const float c2 = 18.8515625;
    const float c3 = 18.6875;

    vec3 e = pow(clamp(v, 0.0, 1.0), vec3(1.0 / m2));
    vec3 num = max(e - c1, 0.0);
    vec3 den = max(c2 - c3 * e, 1e-6);
    return 10000.0 * pow(num / den, vec3(1.0 / m1));
}

// ARIB STD-B67 inverse OETF, to scene light.
vec3 hlg_to_scene(vec3 v) {
    const float a = 0.17883277;
    const float b = 0.28466892;
    const float c = 0.55991073;

    v = clamp(v, 0.0, 1.0);
    return mix(
        v * v / 3.0,
        (exp((v - c) / a) + b) / 12.0,
        step(vec3(0.5), v)
    );
}

// The opto-optical transfer HLG needs, which is why it cannot be per-channel:
// the gain depends on the pixel's luminance, not on each channel alone.
vec3 hlg_to_nits(vec3 v, float peak) {
    vec3 scene = hlg_to_scene(v);
    float luma = dot(scene, vec3(0.2627, 0.6780, 0.0593));
    float gamma = 1.2 + 0.42 * (log2(max(peak, 1.0) / 1000.0) / log2(10.0));
    return scene * (peak * pow(max(luma, 0.0), gamma - 1.0));
}

vec3 decode(vec3 v) {
    if (tf_kind == TF_LINEAR) {
        return v;
    } else if (tf_kind == TF_POWER) {
        return signed_pow(v, tf_param);
    } else if (tf_kind == TF_SRGB_PIECEWISE) {
        return srgb_to_linear(max(v, 0.0));
    } else if (tf_kind == TF_SRGB_EXTENDED) {
        return sign(v) * srgb_to_linear(abs(v));
    } else if (tf_kind == TF_ST240) {
        return mix(
            v / 4.0,
            pow((v + 0.1115) / 1.1115, vec3(1.0 / 0.45)),
            step(vec3(0.0912), v)
        );
    } else if (tf_kind == TF_ST2084_PQ) {
        return pq_to_nits(v);
    } else if (tf_kind == TF_HLG) {
        return hlg_to_nits(v, tf_param);
    } else if (tf_kind == TF_ST428) {
        return (52.37 / 48.0) * pow(max(v, 0.0), vec3(2.6));
    }
    return v;
}

void main() {
    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif

    // Un-premultiply before anything non-linear. Decoding a premultiplied
    // channel applies the curve to `colour × alpha` rather than to the colour,
    // which darkens every translucent pixel — the classic first-HDR bug.
    float a = max(color.a, 1e-5);
    vec3 linear = decode(color.rgb / a) * luminance_scale;
    // Column-major, so this is the matrix times the colour.
    color = vec4(primaries * linear, color.a);
    color.rgb *= color.a;

    vec2 half_size = size * 0.5;
    float limit = min(half_size.x, half_size.y);
    vec4 r = min(radius, vec4(limit));
    float distance = rounded_box(v_coords * size - half_size, half_size, r);

#if defined(GL_OES_standard_derivatives)
    float aa = max(fwidth(distance), 0.0001);
#else
    float aa = 1.0;
#endif
    float mask = 1.0 - smoothstep(-0.5 * aa, 0.5 * aa, distance);

    color = color * (alpha * mask);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
