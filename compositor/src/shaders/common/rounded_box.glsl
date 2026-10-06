
// The one corner shape everything on screen shares: a squircle. Each corner is
// a superellipse of exponent 4 spread over SQUIRCLE_EXTENT times the nominal
// radius, so its curvature eases in from the straight edge instead of jumping
// to a circle's — the continuous corner — while it cuts about as deep as a
// circular corner of that radius. Every shader that draws a rounded thing uses
// this, which is what keeps a window, its border, its shadow and its glass
// concentric.
//
// Concatenated after the shader that uses it, which declares a prototype: GLSL
// ES 1.00 needs a declaration before use, and putting the body last is what
// lets every program share one copy. The extent is mirrored in Rust as
// `utils::squircle::EXTENT` for the CPU side's geometry.
const float SQUIRCLE_EXTENT = 1.5;

// The superellipse norm of a corner-local offset: (x^4 + y^4)^(1/4), with two
// square roots instead of a `pow`.
float squircle_norm(in vec2 q) {
    vec2 q2 = q * q;
    return sqrt(sqrt(dot(q2, q2)));
}

// Signed distance to a box with squircle corners, negative inside. `r` holds
// the radii of the (+x, +y), (+x, -y), (-x, +y) and (-x, -y) corners.
float rounded_box(in vec2 p, in vec2 b, in vec4 r) {
    r.xy = (p.x > 0.0) ? r.xy : r.zw;
    r.x = (p.y > 0.0) ? r.x : r.y;
    float k = min(r.x * SQUIRCLE_EXTENT, min(b.x, b.y));
    vec2 q = abs(p) - b + k;
    return min(max(q.x, q.y), 0.0) + squircle_norm(max(q, 0.0)) - k;
}

// The same with one radius for all four corners.
float rounded_box(in vec2 p, in vec2 b, in float r) {
    return rounded_box(p, b, vec4(r));
}
