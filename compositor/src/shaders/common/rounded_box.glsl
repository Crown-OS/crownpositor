
// Signed distance to a box with per-corner radii, after Inigo Quilez.
// `r` holds the radii of the (+x, +y), (+x, -y), (-x, +y) and (-x, -y) corners.
//
// Concatenated after the shader that uses it, which declares a prototype: GLSL
// ES 1.00 needs a declaration before use, and putting the body last is what
// lets the two programs that need it share one copy.
float rounded_box(in vec2 p, in vec2 b, in vec4 r) {
    r.xy = (p.x > 0.0) ? r.xy : r.zw;
    r.x = (p.y > 0.0) ? r.x : r.y;
    vec2 q = abs(p) - b + r.x;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r.x;
}
