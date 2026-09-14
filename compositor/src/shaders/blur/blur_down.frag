#version 300 es

precision highp float;

uniform sampler2D tex;
// The level being written, in pixels; `gl_FragCoord` is what the pass reads its
// position from, so orientation never enters into it.
uniform vec2 destination_size;
// Half a destination pixel in UV space, and the kawase spread multiplier.
uniform vec2 half_pixel;
uniform float offset;

out vec4 fragment;

// The taps reach past the texture edge by design; clamping half a pixel in
// keeps the edge from bleeding whatever the sampler's border color is.
vec2 cl(vec2 uv) {
    return clamp(uv, half_pixel, vec2(1.0) - half_pixel);
}

// Dual-kawase downsample: 5 taps, center weighted 4, corners 1 each.
// (Marius Bjørge, "Bandwidth-efficient rendering", SIGGRAPH 2015.)
void main() {
    vec2 uv = gl_FragCoord.xy / destination_size;
    vec2 o = half_pixel * offset;

    vec4 sum = texture(tex, cl(uv)) * 4.0;
    sum += texture(tex, cl(uv + vec2(-o.x, -o.y)));
    sum += texture(tex, cl(uv + vec2( o.x, -o.y)));
    sum += texture(tex, cl(uv + vec2(-o.x,  o.y)));
    sum += texture(tex, cl(uv + vec2( o.x,  o.y)));

    fragment = sum / 8.0;
}
