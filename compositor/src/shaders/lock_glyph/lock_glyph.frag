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

// Element UV to sprite UV: undoes the clock's warp, so the element can be
// the box around the warped glyph and the glyph still lands in perspective.
uniform mat3 unwarp;
// The blur radius in sprite UV units, per axis; zero draws sharp.
uniform vec2 reach;

const int TAPS = 48;
const float GOLDEN_ANGLE = 2.39996323;

#if defined(GLASS)
// The output's blur pyramid, top level, addressed by `gl_FragCoord` like the
// rest of the compositor's glass: frosted digits show the background through.
uniform sampler2D scene_tex;
uniform vec2 scene_size;
uniform vec2 half_pixel;
uniform float offset;
// ColorOS 17's glass, as `lockscreen::clock::Glass` fits it: the frost times
// `gain` — and whatever undoes the darkening the pyramid lifted with it —
// plus `white`, plus `top_shine` fading out `top_shine_reach` of the
// way down, plus `rim_white` along the inner rim, clamped. On the stored,
// sRGB-encoded values, as Skia blends plus-lighter.
uniform float gain;
uniform float white;
uniform float top_shine;
uniform float top_shine_reach;
uniform float rim_white;
// The sprite rows the glyph's top and baseline sit on.
uniform vec2 rows;
// How deep the inner rim reaches, `lockscreen::clock::rim_reach`, in sprite
// UV per axis.
uniform vec2 rim_reach;

vec2 cl(vec2 uv) {
    return clamp(uv, half_pixel, vec2(1.0) - half_pixel);
}

// Dual-kawase upsample, as the compositor's other glass finishes its blur.
vec3 upsample(vec2 uv) {
    vec2 o = half_pixel * offset;
    vec4 sum = vec4(0.0);
    sum += texture2D(scene_tex, cl(uv + vec2(-o.x * 2.0, 0.0)));
    sum += texture2D(scene_tex, cl(uv + vec2( o.x * 2.0, 0.0)));
    sum += texture2D(scene_tex, cl(uv + vec2(0.0, -o.y * 2.0)));
    sum += texture2D(scene_tex, cl(uv + vec2(0.0,  o.y * 2.0)));
    sum += texture2D(scene_tex, cl(uv + vec2(-o.x,  o.y))) * 2.0;
    sum += texture2D(scene_tex, cl(uv + vec2( o.x,  o.y))) * 2.0;
    sum += texture2D(scene_tex, cl(uv + vec2(-o.x, -o.y))) * 2.0;
    sum += texture2D(scene_tex, cl(uv + vec2( o.x, -o.y))) * 2.0;
    return (sum / 12.0).rgb;
}
#endif

vec4 glyph(vec2 uv) {
    if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0)
        return vec4(0.0);
    return texture2D(tex, uv);
}

void main() {
    vec3 mapped = unwarp * vec3(v_coords, 1.0);
    if (mapped.z <= 0.0) {
        gl_FragColor = vec4(0.0);
        return;
    }
    vec2 uv = mapped.xy / mapped.z;

    vec4 color;
    if (reach.x <= 0.0 && reach.y <= 0.0) {
        color = glyph(uv);
    } else {
        // The same golden-angle spiral as a blurring lock layer.
        vec4 sum = vec4(0.0);
        for (int i = 0; i < TAPS; i++) {
            float t = (float(i) + 0.5) / float(TAPS);
            float angle = float(i) * GOLDEN_ANGLE;
            sum += glyph(uv + vec2(cos(angle), sin(angle)) * sqrt(t) * reach);
        }
        color = sum / float(TAPS);
    }

#if defined(GLASS)
    // The sprite is the glyph's coverage in white; the colour is the glass.
    float coverage = color.a;
    float down = clamp((uv.y - rows.x) / max(rows.y - rows.x, 0.0001), 0.0, 1.0);
    // `lockscreen::clock::rim_at`, which the preview fills its glass with.
    float eroded = min(
        min(glyph(uv + vec2(rim_reach.x, 0.0)).a, glyph(uv - vec2(rim_reach.x, 0.0)).a),
        min(glyph(uv + vec2(0.0, rim_reach.y)).a, glyph(uv - vec2(0.0, rim_reach.y)).a));
    float rim = clamp(coverage - eroded, 0.0, 1.0);
    vec3 frost = upsample(gl_FragCoord.xy / scene_size);
    float lift = white
        + top_shine * (1.0 - smoothstep(0.0, top_shine_reach, down))
        + rim_white * rim;
    vec3 face = min(frost * gain + lift, vec3(1.0));
    color = vec4(face * coverage, coverage);
#endif

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
