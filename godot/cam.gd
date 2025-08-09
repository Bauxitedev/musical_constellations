extends Node3D

@onready var cam := $Camera3D
@onready var cam_start_pos = cam.position
@onready var cam_end_pos = cam.position * 10.0

# Rotation speed in degrees per second
var max_rot_speed := 90.0
var rot_speed_hor := 0.0
var rot_speed_vert := 0.0

# Limits for up/down rotation (in degrees)
@export var min_pitch := -89.9
@export var max_pitch := 89.9

var pitch := 0.0 # Current up/down angle
var zoom_amount := 0.0
var zoom_speed := -0.3

var rotating = {
	"camera_left": false,
	"camera_right": false,
	"camera_up": false,
	"camera_down": false,
}

func _unhandled_input(event):
	# Using _unhandled_input so we don't rotate while the user is typing in the textbox
	if event is InputEventKey:
		for action in rotating.keys():
			if event.is_action_pressed(action):
				rotating[action] = true
			elif event.is_action_released(action):
				rotating[action] = false

	if event.is_action_pressed("camera_zoom"):
		zoom_speed = - zoom_speed

func _process(delta):
	var lerp_speed := 4.0

	var rot_h := 0.0
	var rot_v := 0.0

	# Tank controls
	if rotating["camera_left"]:
		rot_h += 1.0
	if rotating["camera_right"]:
		rot_h -= 1.0
	if rotating["camera_up"]:
		rot_v += 1.0
	if rotating["camera_down"]:
		rot_v -= 1.0

	# Invert controls if we're outside the sphere
	var rot_speed_zoom_mult = 1.0 if zoom_amount < 0.5 else -1.0

	# Rotate
	rot_speed_vert = Util.lerp_smooth_float(rot_speed_vert, rot_v * max_rot_speed * rot_speed_zoom_mult, lerp_speed, delta)
	pitch += rot_speed_vert * delta
	pitch = clamp(pitch, min_pitch, max_pitch)
	rotation.x = deg_to_rad(pitch)

	rot_speed_hor = Util.lerp_smooth_float(rot_speed_hor, rot_h * max_rot_speed * rot_speed_zoom_mult, lerp_speed, delta)
	rotation.y += deg_to_rad(rot_speed_hor * delta)

	# Zoom
	zoom_amount = clamp(zoom_amount + zoom_speed * delta, 0.0, 1.0)
	cam.position = cam_start_pos.lerp(cam_end_pos, smoothstep(0.0, 1.0, zoom_amount))
