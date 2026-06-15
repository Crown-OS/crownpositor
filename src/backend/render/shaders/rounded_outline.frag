precision highp float;
uniform float alpha;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform vec2 size;
varying vec2 v_coords;

uniform vec3 color;
uniform float thickness;
uniform vec4 radius;
uniform float scale;

float sd_rounded_rect(vec2 p, vec2 sz, vec4 r) {
    vec2 half_sz = sz * 0.5;
    vec2 q = p - half_sz;
    float cr;
    if (q.x <= 0.0) {
        cr = (q.y <= 0.0) ? r.x : r.w;
    } else {
        cr = (q.y <= 0.0) ? r.y : r.z;
    }
    vec2 d = abs(q) - half_sz + vec2(cr);
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2(0.0))) - cr;
}

void main() {
    vec2 location = v_coords * size;
    float half_px = 0.5 / scale;

    float outer_sdf = sd_rounded_rect(location, size, radius);
    float outer_alpha = 1.0 - smoothstep(-half_px, half_px, outer_sdf);

    float inner_alpha = 1.0;
    if (thickness > 0.0) {
        vec2 inner_size = size - vec2(thickness * 2.0);
        vec2 inner_loc = location - vec2(thickness);
        vec4 inner_radius = max(radius - vec4(thickness), vec4(0.0));
        float inner_sdf = sd_rounded_rect(inner_loc, inner_size, inner_radius);
        inner_alpha = smoothstep(-half_px, half_px, inner_sdf);
    }

    float border_alpha = outer_alpha * inner_alpha;
    vec4 mix_color = vec4(color, alpha) * border_alpha;

    #if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        mix_color = vec4(0.0, 0.3, 0.0, 0.2) + mix_color * 0.8;
    #endif

    gl_FragColor = mix_color;
}
