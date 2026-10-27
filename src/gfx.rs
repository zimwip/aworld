//! Everything the game draws with, reads the keyboard and the mouse with, and opens its window
//! with, in one place: the rest of the game only sees what is listed here. For now it all comes
//! from macroquad; it is the one file to change to draw with wgpu and open the window with winit.

// Maths and colours.
pub use macroquad::prelude::{vec2, vec3, vec4, Color, Rect, Vec2, BLACK, BLANK, ORANGE, WHITE};

// The window, the frame and the clock.
pub use macroquad::prelude::{
    clear_background, get_frame_time, get_time, next_frame, screen_height, screen_width, set_fullscreen, show_mouse, Conf,
};

// Keyboard and mouse.
pub use macroquad::prelude::{
    get_keys_pressed, is_key_down, is_key_pressed, is_mouse_button_down, is_mouse_button_pressed, is_mouse_button_released,
    mouse_position, mouse_wheel, KeyCode, MouseButton,
};

// Shapes.
pub use macroquad::prelude::{
    draw_arc, draw_circle, draw_circle_lines, draw_ellipse, draw_line, draw_mesh, draw_poly, draw_rectangle, draw_rectangle_ex,
    draw_rectangle_lines, draw_triangle, DrawRectangleParams, Mesh, Vertex,
};

// Text.
pub use macroquad::prelude::{draw_text, draw_text_ex, load_ttf_font_from_bytes, measure_text, Font, TextParams};

// Images, textures and the cameras that draw into them.
pub use macroquad::prelude::{
    draw_texture_ex, render_target, set_camera, set_default_camera, Camera2D, DrawTextureParams, FilterMode, Image, RenderTarget,
    Texture2D,
};

// Shaders.
pub use macroquad::miniquad::{BlendFactor, BlendState, BlendValue, Equation, PipelineParams};
pub use macroquad::prelude::{
    gl_use_default_material, gl_use_material, load_material, Material, MaterialParams, ShaderSource, UniformDesc, UniformType,
};

/// Moves everything drawn until `pop_shift` right by `dx` pixels.
pub fn push_shift(dx: f32) {
    use macroquad::prelude::{get_internal_gl, Mat4};
    unsafe { get_internal_gl() }.quad_gl.push_model_matrix(Mat4::from_translation(vec3(dx, 0.0, 0.0)));
}

pub fn pop_shift() {
    unsafe { macroquad::prelude::get_internal_gl() }.quad_gl.pop_model_matrix();
}

/// Writes what the window shows to a PNG file.
pub fn save_screenshot(path: &str) {
    macroquad::prelude::get_screen_data().export_png(path);
}

/// Asks the window to close (in the browser, the page goes back to the game).
#[cfg(target_arch = "wasm32")]
pub fn request_quit() {
    macroquad::miniquad::window::request_quit();
}
