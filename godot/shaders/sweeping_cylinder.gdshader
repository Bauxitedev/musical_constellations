// NOTE: Shader automatically converted from Godot Engine 4.4.1.stable's StandardMaterial3D.

shader_type spatial;
render_mode blend_add, cull_back, unshaded;

uniform vec4 albedo : source_color;

varying vec3 local_vertex_pos;
varying float progress; //0 to 1


void vertex() {
	UV = UV; //not sure if needed
	local_vertex_pos = VERTEX; // VERTEX is in local space
	progress = INSTANCE_CUSTOM.r;
}

void fragment() {
	vec2 base_uv = UV;

	float target_y = progress - 0.5; //NOTE - the cylinder is defined from Y = -0.5 to 0.5
	float current_y = local_vertex_pos.y;
	float diff_y = abs(current_y - target_y);
	float brightness_mult = max(30.0 / (diff_y * 100.0 + 1.0), 0.5); //Don't make this too bright or it messes with MSAA

	// Vertex Color Use as Albedo: Enabled
	// TODO maybe need to interpret as sRGB here?
	ALBEDO = albedo.rgb * COLOR.rgb * brightness_mult;
	ALPHA *= albedo.a * COLOR.a;
}
