
// The colour-converting variant's half of `rounded_corner.frag`: a surface's
// own encoding to the output's 8-bit SDR one. Every constant here has a CPU
// reference in `color::transfer` and `color::tone_map`.

// Which curve to decode with. Must match `color::pipeline::TransferKind`.
uniform int tf_kind;
// The power curve's exponent, or HLG's peak luminance in cd/m².
uniform float tf_param;
// Brings decoded light onto "1.0 is reference white".
uniform float luminance_scale;
// The content's peak over its reference white.
uniform float headroom;
// Source primaries onto the output's, column-major.
uniform mat3 primaries;

#define TF_LINEAR         0
#define TF_POWER          1
#define TF_SRGB_PIECEWISE 2
#define TF_SRGB_EXTENDED  3
#define TF_ST240          4
#define TF_ST2084_PQ      5
#define TF_HLG            6
#define TF_ST428          7

const float TONE_KNEE = 0.75;
const float OUTPUT_GAMMA = 2.2;

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

// HLG's opto-optical transfer is cross-channel: the gain depends on the
// pixel's luminance, not on each channel alone.
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

// Identity up to the knee, then an extended Reinhard shoulder joined with
// matching slope that lands the content's peak on 1.0. Driven by the
// brightest channel so all three scale alike and hue survives.
vec3 tone_map(vec3 light) {
    float brightest = max(max(light.r, light.g), light.b);
    if (headroom <= 1.0 || brightest <= TONE_KNEE) {
        return light;
    }
    float span = 1.0 - TONE_KNEE;
    float over = (brightest - TONE_KNEE) / span;
    float white = (headroom - TONE_KNEE) / span;
    float mapped = TONE_KNEE + span * over * (1.0 + over / (white * white)) / (1.0 + over);
    return light * (mapped / brightest);
}

// Un-premultiplied before anything non-linear: a curve applied to
// `colour × alpha` darkens every translucent pixel.
vec4 convert_color(vec4 premultiplied) {
    float a = premultiplied.a;
    if (a <= 0.0) {
        return vec4(0.0);
    }
    vec3 light = primaries * (decode(premultiplied.rgb / a) * luminance_scale);
    light = tone_map(max(light, 0.0));
    vec3 encoded = pow(clamp(light, 0.0, 1.0), vec3(1.0 / OUTPUT_GAMMA));
    return vec4(encoded * a, a);
}
