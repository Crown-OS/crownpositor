#version 300 es

// A window's content frosted while it changes size. The blur deepens with
// `strength` by reading ever coarser mip levels, and a golden-angle spiral of
// taps one level's texel wide smooths the blocks a mip alone would show. At
// strength 0 it reads the base level in one place: the content, untouched.

precision highp float;

const float MAX_LOD = 6.0;
const float SPREAD = 2.0;
const int TAPS = 16;
const float GOLDEN_ANGLE = 2.39996323;

uniform sampler2D tex;
uniform float alpha;
uniform vec4 window;
// Blur strength from 0 to 1, then the corner radius in physical pixels.
uniform vec2 blur;

in vec2 v_uv;

out vec4 color;

float rounded_box(in vec2 p, in vec2 b, in float r);

void main() {
    float d = rounded_box((v_uv - 0.5) * window.zw, window.zw * 0.5, blur.y);
    float coverage = clamp(0.5 - d / max(fwidth(d), 1e-4), 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }

    float lod = blur.x * MAX_LOD;
    vec2 reach = SPREAD * (exp2(lod) - 1.0) / vec2(textureSize(tex, 0));
    vec4 sum = vec4(0.0);
    for (int i = 0; i < TAPS; i++) {
        float t = (float(i) + 0.5) / float(TAPS);
        float angle = float(i) * GOLDEN_ANGLE;
        sum += textureLod(tex, v_uv + vec2(cos(angle), sin(angle)) * sqrt(t) * reach, lod);
    }
    color = sum / float(TAPS) * (coverage * alpha);
}
