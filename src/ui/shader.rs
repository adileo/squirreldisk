//! OpenGL renderer for the sunburst segments.
//!
//! Each segment is an annular sector tessellated on the CPU; the fragment shader
//! does analytic anti-aliased gaps, a thin edge rim, a hover tint and a hatch
//! pattern for items that are waiting in the collector.

use super::sunburst::VERT_FLOATS;
use eframe::glow::{self, HasContext};
use std::sync::Arc;

const VS: &str = r#"
#ifdef NEW_SHADER_INTERFACE
  #define I in
  #define O out
#else
  #define I attribute
  #define O varying
#endif
uniform vec2 u_screen;
I vec2 a_pos;
I vec4 a_color;
I vec4 a_polar;
I vec4 a_params;
O vec4 v_color;
O vec4 v_polar;
O vec4 v_params;
O vec2 v_pos;
void main() {
    v_color = a_color;
    v_polar = a_polar;
    v_params = a_params;
    v_pos = a_pos;
    gl_Position = vec4(2.0 * a_pos.x / u_screen.x - 1.0, 1.0 - 2.0 * a_pos.y / u_screen.y, 0.0, 1.0);
}
"#;

const FS: &str = r#"
#ifdef GL_ES
precision highp float;
#endif
#ifdef NEW_SHADER_INTERFACE
  #define I in
  out vec4 f_color;
  #define FRAG f_color
#else
  #define I varying
  #define FRAG gl_FragColor
#endif
uniform float u_time;
uniform float u_ppp;
uniform vec2 u_center;
uniform float u_fx;
uniform vec3 u_light;
I vec4 v_color;
I vec4 v_polar;
I vec4 v_params;
I vec2 v_pos;

float hash(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

bool has_flag(float flags, float f) {
    return mod(floor(flags / f), 2.0) > 0.5;
}

void main() {
    float hover = v_params.x;
    float flags = v_params.y;
    float r = v_params.z;
    float seed = v_params.w;
    bool center = has_flag(flags, 4.0);

    // distance to the four edges, in points
    float d_side = min(v_polar.x, v_polar.y) * r;
    float d_rad = min(v_polar.z, v_polar.w);
    float d = center ? v_polar.w : min(d_side, d_rad);

    // analytic anti-aliasing with a thin gap between segments
    float gap = center ? 0.0 : 0.55;
    float px = 1.0 / u_ppp;
    float aa = smoothstep(gap - px * 0.5, gap + px * 0.9, d);

    vec3 c = v_color.rgb;
    if (u_fx > 0.5) {
        // Flat fill with a crisp, very thin inner rim so neighbours read as
        // separate tiles without any visible gradient across the ring.
        float rim = 1.0 - smoothstep(gap + 0.4, gap + 1.6, d);
        c = mix(c, c * 1.12 + 0.02, rim * 0.6);
        if (center) {
            c *= 0.97;
        }
    }

    // hover: colour change only, geometry and edges stay put
    c = mix(c, vec3(1.0), 0.22 * hover);

    // items already in the collector: desaturated with moving hatch
    if (has_flag(flags, 1.0)) {
        float g = dot(c, vec3(0.299, 0.587, 0.114));
        c = mix(c, vec3(g), 0.75) * 0.7;
        float stripe = step(0.5, fract((gl_FragCoord.x + gl_FragCoord.y) / (9.0 * u_ppp) - u_time * 0.4));
        c = mix(c, c * 1.35, stripe * 0.5);
    }
    // item being dragged: dim ghost
    if (has_flag(flags, 2.0)) {
        c *= 0.55;
    }
    // aggregates: subtle dotted texture


    float alpha = v_color.a * aa;
    FRAG = vec4(clamp(c, 0.0, 1.0) * alpha, alpha);
}
"#;

pub struct SunburstGl {
    program: glow::Program,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    vao: glow::VertexArray,
    u_screen: Option<glow::UniformLocation>,
    u_time: Option<glow::UniformLocation>,
    u_ppp: Option<glow::UniformLocation>,
    u_center: Option<glow::UniformLocation>,
    u_fx: Option<glow::UniformLocation>,
    u_light: Option<glow::UniformLocation>,
}

pub struct Frame {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
    pub screen: [f32; 2],
    pub center: [f32; 2],
    pub time: f32,
    pub fx: bool,
}

impl SunburstGl {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        let sv = egui_glow::ShaderVersion::get(gl);
        let header = format!(
            "{}\n{}",
            sv.version_declaration(),
            if sv.is_new_shader_interface() { "#define NEW_SHADER_INTERFACE\n" } else { "" }
        );
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (ty, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
                let s = gl.create_shader(ty)?;
                gl.shader_source(s, &format!("{header}{src}"));
                gl.compile_shader(s);
                if !gl.get_shader_compile_status(s) {
                    return Err(format!("shader compile error: {}", gl.get_shader_info_log(s)));
                }
                gl.attach_shader(program, s);
                shaders.push(s);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(format!("shader link error: {}", gl.get_program_info_log(program)));
            }
            for s in shaders {
                gl.detach_shader(program, s);
                gl.delete_shader(s);
            }
            let vbo = gl.create_buffer()?;
            let ebo = gl.create_buffer()?;
            let vao = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = (VERT_FLOATS * 4) as i32;
            for (name, size, offset) in [("a_pos", 2, 0), ("a_color", 4, 2), ("a_polar", 4, 6), ("a_params", 4, 10)] {
                if let Some(loc) = gl.get_attrib_location(program, name) {
                    gl.enable_vertex_attrib_array(loc);
                    gl.vertex_attrib_pointer_f32(loc, size, glow::FLOAT, false, stride, offset * 4);
                }
            }
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            Ok(SunburstGl {
                u_screen: gl.get_uniform_location(program, "u_screen"),
                u_time: gl.get_uniform_location(program, "u_time"),
                u_ppp: gl.get_uniform_location(program, "u_ppp"),
                u_center: gl.get_uniform_location(program, "u_center"),
                u_fx: gl.get_uniform_location(program, "u_fx"),
                u_light: gl.get_uniform_location(program, "u_light"),
                program,
                vbo,
                ebo,
                vao,
            })
        }
    }

    pub fn paint(&self, gl: &glow::Context, f: &Frame, ppp: f32) {
        if f.indices.is_empty() {
            return;
        }
        unsafe {
            gl.use_program(Some(self.program));
            gl.uniform_2_f32(self.u_screen.as_ref(), f.screen[0], f.screen[1]);
            gl.uniform_2_f32(self.u_center.as_ref(), f.center[0], f.center[1]);
            gl.uniform_1_f32(self.u_time.as_ref(), f.time);
            gl.uniform_1_f32(self.u_ppp.as_ref(), ppp);
            gl.uniform_1_f32(self.u_fx.as_ref(), if f.fx { 1.0 } else { 0.0 });
            gl.uniform_3_f32(self.u_light.as_ref(), -0.4, -0.9, 0.0);
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck_f32(&f.vertices), glow::STREAM_DRAW);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.ebo));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, bytemuck_u32(&f.indices), glow::STREAM_DRAW);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func_separate(glow::ONE, glow::ONE_MINUS_SRC_ALPHA, glow::ONE_MINUS_DST_ALPHA, glow::ONE);
            gl.draw_elements(glow::TRIANGLES, f.indices.len() as i32, glow::UNSIGNED_INT, 0);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_buffer(self.vbo);
            gl.delete_buffer(self.ebo);
            gl.delete_vertex_array(self.vao);
        }
    }
}

fn bytemuck_f32(v: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}
fn bytemuck_u32(v: &[u32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

/// Builds the egui paint callback for one frame.
pub fn callback(renderer: Arc<std::sync::Mutex<SunburstGl>>, frame: Arc<Frame>, rect: eframe::egui::Rect) -> eframe::egui::PaintCallback {
    eframe::egui::PaintCallback {
        rect,
        callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
            if let Ok(r) = renderer.lock() {
                r.paint(painter.gl(), &frame, info.pixels_per_point);
            }
        })),
    }
}
