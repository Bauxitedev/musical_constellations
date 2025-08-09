class_name Util extends Node

static func lerp_smooth_float(a: Variant, b: Variant, lerp_speed: float, delta: float) -> Variant:
	var t = clamp(1.0 - exp(-delta * lerp_speed), 0.0, 1.0)
	return lerp(a, b, t)
	
static func lerp_smooth(a: Variant, b: Variant, lerp_speed: float, delta: float) -> Variant:
	var t = clamp(1.0 - exp(-delta * lerp_speed), 0.0, 1.0)
	return a.lerp(b, t)

static func show_and_wait_accept_dialog(message: String) -> void:
	var dialog := AcceptDialog.new()
	dialog.dialog_text = message
	Engine.get_main_loop().root.add_child(dialog)
	dialog.popup_centered()
	
	await dialog.visibility_changed # Awaits both clicking OK and X
	dialog.queue_free() # Optional: remove dialog after use
	print("User accepted/closed the dialog")

	
