#version 300 es

// A single oversized triangle covering clip space, built from the vertex index
// alone: the kawase passes bind no buffers and enable no attribute arrays.
void main() {
    vec2 corner = vec2(float((gl_VertexID & 1) << 2), float((gl_VertexID & 2) << 1)) - 1.0;
    gl_Position = vec4(corner, 0.0, 1.0);
}
