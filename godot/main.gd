extends WorldEnvironment

@onready var audio_world_holder := $AudioWorldHolder
@onready var ui := $UI
@onready var ui_control := $UIControl
@onready var ui_control_margin := $UIControl/MarginContainer
@onready var intro_text := $UIControl/IntroText
@onready var seed_label := $UIControl/MarginContainer/VBoxContainer/HBoxContainer2/SeedEdit
@onready var bpm_label := $UIControl/MarginContainer/VBoxContainer/HBoxContainer/BPMLabel
@onready var bpm_hslider := $UIControl/MarginContainer/VBoxContainer/HBoxContainer/BPMHSlider
@onready var perf_label := $UIControl/MarginContainer/PerfLabel
@onready var version_label := $UIControl/MarginContainer/VBoxContainer/TabContainer/About/VersionLabel
@onready var debug_label := $UIControl/MarginContainer/VBoxContainer/TabContainer/Statistics/DebugLabel
@onready var controls_tab := $UIControl/MarginContainer/VBoxContainer/TabContainer/Controls

func _ready():
	# set fullscreen in exported game, and if not disabled
	if !GlobalCliArgs.windowed:
		DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_FULLSCREEN)

	# setup signals
	GlobalAudioState.bpm_changed.connect(_on_bpm_changed)
	GlobalAudioState.seed_changed.connect(_on_seed_changed)
	GlobalAudioState.graph_debug_str_changed.connect(_on_graph_debug_str_changed)

	# update UI
	update_slider()
	update_bpm_label()
	update_seed_label()
	update_version_label()

	# ensure the first tab is shown, regardless of the one that's open in the editor
	controls_tab.show()

	# load the actual graph
	reload_audio_world()

	# play intro animation
	play_intro_animation()

func play_intro_animation():
	var dur_mult := 0.1 if GlobalCliArgs.skip_intro else 1.0

	ui.modulate = Color.TRANSPARENT
	ui_control_margin.modulate = Color.TRANSPARENT

	var tween := get_tree().create_tween()
	tween.tween_property(intro_text, "modulate", Color.WHITE, 3 * dur_mult).from(Color.TRANSPARENT)
	tween.tween_property(ui, "modulate", Color.WHITE, 1 * dur_mult).from(Color.TRANSPARENT)
	tween.tween_property(ui_control_margin, "modulate", Color.WHITE, 3 * dur_mult).from(Color.TRANSPARENT)
	tween.tween_property(intro_text, "modulate", Color.TRANSPARENT, 1.5 * dur_mult)

	await get_tree().create_timer(5.0 * dur_mult).timeout

func reload_audio_world():
	var start_time := Time.get_ticks_usec()

	# in case the user presses Q but not enter
	update_seed_label()

	# TODO removing these nodes this takes up like 48ms!!!
	for c in audio_world_holder.get_children():
		print("unloading ", c)
		c.queue_free()

	var world := preload("res://scenes/audio_world.tscn")
	var node := world.instantiate()
	audio_world_holder.add_child(node)

	var end_time := Time.get_ticks_usec()
	var duration := (end_time - start_time) / 1000.0 # convert to ms
	print("reloading audio world took ", duration, "ms")

func _process(delta):
	update_bpm_label()

	# Update every 11 frames
	if Engine.get_frames_drawn() % 11 == 0:
		perf_label.text = GlobalAudioState.get_perf_str()

func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventMouseButton:
		get_viewport().gui_release_focus()
		# unfocuses the label and other stuff

	if event.is_action_pressed("restart_same_seed"):
		reload_audio_world()

	if event.is_action_pressed("toggle_fullscreen"):
		DisplayServer.window_set_mode(
			DisplayServer.WINDOW_MODE_WINDOWED if DisplayServer.window_get_mode() == DisplayServer.WINDOW_MODE_FULLSCREEN
			else DisplayServer.WINDOW_MODE_FULLSCREEN
		)

	# Weird - normally event.pressed would prevent this from being triggered while you're focused on a textbox.
	# Maybe it only works with keys that actually produce characters in the textbox?
	if event.is_action_pressed("toggle_ui"):
		ui_control_margin.visible = !ui_control_margin.visible

func _on_bpm_hslider_value_changed(value):
	# TODO this is also emitted if `value` is changed via code instead of the user. It messes with BPM tap, it's forcibly rounding the BPM value!!!
	# Try to find a signal that is only emitted if the USER changes the slider.
	# See https://godotforums.org/d/41062-how-to-tell-if-a-hsliders-value-was-changed-by-mouse-input-or-by-code
	print("_on_bpm_hslider_value_changed -> ", value)
	GlobalAudioState.set_bpm(value)

func _on_seed_edit_text_submitted(text: String):
	var result = GlobalAudioState.set_seed_str(text)
	if result:
		reload_audio_world()
	else:
		Util.show_and_wait_accept_dialog("Invalid seed, must be 16 hexadecimal characters (e.g. DEADBEEFDEADBEEF)")

func _on_bpm_changed(bpm: float):
	update_slider()

func _on_seed_changed(seed: int):
	update_seed_label()

func _on_graph_debug_str_changed(graph_debug_str: String):
	update_graph_debug_str_label()

func _on_randomize_button_pressed():
	GlobalAudioState.randomize_seed()
	reload_audio_world()

#########

func update_slider():
	bpm_hslider.value = GlobalAudioState.bpm
	# Don't call this every frame or you can't slide it manually anymore

func update_bpm_label():
	var bpm = int(round(GlobalAudioState.bpm))
	var bpm_str = "%3d" % bpm # Format with width = 3, right-aligned
	bpm_label.text = "BPM: %s" % bpm_str

func update_seed_label():
	seed_label.text = "%s" % GlobalAudioState.get_seed_str()

func update_graph_debug_str_label():
	debug_label.text = GlobalAudioState.get_debug_str()

func update_version_label():
	version_label.text = GlobalAudioState.get_version_str()
