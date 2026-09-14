#version 300 es

precision highp float;

uniform sampler2D tex;
uniform vec2 destination_size;
// Half a *source* pixel in UV space, and the kawase spread multiplier.
uniform vec2 half_pixel;
uniform float offset;

out vec4 fragment;

vec2 cl(vec2 uv) {
    return clamp(uv, half_pixel, vec2(1.0) - half_pixel);
}

// Dual-kawase upsample: 8 taps, diagonals weighted 2, edges 1.
void main() {
    vec2 uv = gl_FragCoord.xy / destination_size;
    vec2 o = half_pixel * offset;

    vec4 sum = vec4(0.0);

    // Four edge centers.
    sum += texture(tex, cl(uv + vec2(-o.x * 2.0, 0.0)));
    sum += texture(tex, cl(uv + vec2( o.x * 2.0, 0.0)));
    sum += texture(tex, cl(uv + vec2(0.0, -o.y * 2.0)));
    sum += texture(tex, cl(uv + vec2(0.0,  o.y * 2.0)));

    // Four diagonals.
    sum += texture(tex, cl(uv + vec2(-o.x,  o.y))) * 2.0;
    sum += texture(tex, cl(uv + vec2( o.x,  o.y))) * 2.0;
    sum += texture(tex, cl(uv + vec2(-o.x, -o.y))) * 2.0;
    sum += texture(tex, cl(uv + vec2( o.x, -o.y))) * 2.0;

    fragment = sum / 12.0;
}
