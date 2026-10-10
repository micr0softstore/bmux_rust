//! Apply the window silhouette after every UI layer has been painted.
use std::sync::Arc;

use eframe::{egui, egui_glow, glow};
use glow::HasContext as _;

const CORNER_RADIUS: f32 = 12.0;

pub struct CornerMask {
    program: glow::Program,
    vertices: glow::Buffer,
    vao: Option<glow::VertexArray>,
    size: glow::UniformLocation,
    radius: glow::UniformLocation,
}

impl CornerMask {
    pub fn install(cc: &eframe::CreationContext<'_>) -> anyhow::Result<Arc<Self>> {
        let gl = cc
            .gl
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("OpenGL is unavailable"))?;
        let mask = Arc::new(Self::new(gl)?);
        let paint_mask = mask.clone();
        cc.egui_ctx.add_plugin(WindowShape {
            rect: None,
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                paint_mask.paint(painter.gl(), info.screen_size_px, info.pixels_per_point);
            })),
        });
        Ok(mask)
    }

    fn new(gl: &glow::Context) -> anyhow::Result<Self> {
        let version = egui_glow::ShaderVersion::get(gl);
        let header = version.version_declaration();
        let precision = if version.is_embedded() {
            "precision highp float;\n"
        } else {
            ""
        };
        let attribute = if version.is_new_shader_interface() {
            "in"
        } else {
            "attribute"
        };
        let (output, color) = if version.is_new_shader_interface() {
            ("out vec4 out_color;", "out_color")
        } else {
            ("", "gl_FragColor")
        };
        let vertex = format!(
            "{header}{precision}{attribute} vec2 a_position;\nvoid main() {{ gl_Position = vec4(a_position, 0.0, 1.0); }}"
        );
        // GLES2 does not require high precision in fragment shaders.
        let fragment_precision = if version.is_embedded() {
            "#ifdef GL_FRAGMENT_PRECISION_HIGH\nprecision highp float;\n#else\nprecision mediump float;\n#endif\n"
        } else {
            ""
        };
        let fragment = format!(
            "{header}{fragment_precision}{output}\n\
             uniform vec2 u_size;\n\
             uniform float u_radius;\n\
             void main() {{\n\
                 vec2 edge = min(gl_FragCoord.xy, u_size - gl_FragCoord.xy);\n\
                 vec2 corner = max(vec2(u_radius) - edge, vec2(0.0));\n\
                 float coverage = clamp(u_radius - length(corner) + 0.5, 0.0, 1.0);\n\
                 {color} = vec4(coverage);\n\
             }}"
        );

        // All resource creation and drawing occur on eframe's current GL context.
        unsafe {
            let program = gl.create_program().map_err(anyhow::Error::msg)?;
            let linked = (|| -> anyhow::Result<()> {
                for (kind, source) in [
                    (glow::VERTEX_SHADER, vertex),
                    (glow::FRAGMENT_SHADER, fragment),
                ] {
                    let shader = gl.create_shader(kind).map_err(anyhow::Error::msg)?;
                    gl.shader_source(shader, &source);
                    gl.compile_shader(shader);
                    let compiled = gl.get_shader_compile_status(shader);
                    let log = gl.get_shader_info_log(shader);
                    if compiled {
                        gl.attach_shader(program, shader);
                    }
                    gl.delete_shader(shader);
                    anyhow::ensure!(compiled, "Window corner shader: {log}");
                }
                gl.bind_attrib_location(program, 0, "a_position");
                gl.link_program(program);
                anyhow::ensure!(
                    gl.get_program_link_status(program),
                    "Window corner shader: {}",
                    gl.get_program_info_log(program)
                );
                Ok(())
            })();
            if let Err(error) = linked {
                gl.delete_program(program);
                return Err(error);
            }
            let uniforms = gl
                .get_uniform_location(program, "u_size")
                .zip(gl.get_uniform_location(program, "u_radius"));
            let Some((size, radius)) = uniforms else {
                gl.delete_program(program);
                anyhow::bail!("Window corner shader uniforms are unavailable");
            };
            let vertices = match gl.create_buffer() {
                Ok(buffer) => buffer,
                Err(error) => {
                    gl.delete_program(program);
                    return Err(anyhow::Error::msg(error));
                }
            };
            let supports_vao = gl.version().major >= 3
                || [
                    "GL_ARB_vertex_array_object",
                    "ARB_vertex_array_object",
                    "GL_OES_vertex_array_object",
                    "OES_vertex_array_object",
                ]
                .iter()
                .any(|name| gl.supported_extensions().contains(*name));
            let vao = if supports_vao {
                match gl.create_vertex_array() {
                    Ok(vao) => Some(vao),
                    Err(error) => {
                        gl.delete_buffer(vertices);
                        gl.delete_program(program);
                        return Err(anyhow::Error::msg(error));
                    }
                }
            } else {
                None
            };
            let triangle: Vec<u8> = [-1.0_f32, -1.0, 3.0, -1.0, -1.0, 3.0]
                .into_iter()
                .flat_map(f32::to_ne_bytes)
                .collect();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertices));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &triangle, glow::STATIC_DRAW);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            Ok(Self {
                program,
                vertices,
                vao,
                size,
                radius,
            })
        }
    }

    fn paint(&self, gl: &glow::Context, [width, height]: [u32; 2], pixels_per_point: f32) {
        let radius = (CORNER_RADIUS * pixels_per_point).min(width.min(height) as f32 / 2.0);
        if radius <= 0.0 || !radius.is_finite() {
            return;
        }
        // Multiply both destination RGB and alpha by coverage. Painting a
        // transparent rectangle with ordinary alpha blending cannot erase it.
        // Keep eframe's framebuffer bound; egui restores its other drawing state
        // immediately after the callback, including its own VAO and blending.
        unsafe {
            gl.viewport(0, 0, width as i32, height as i32);
            gl.use_program(Some(self.program));
            gl.uniform_2_f32(Some(&self.size), width as f32, height as f32);
            gl.uniform_1_f32(Some(&self.radius), radius);
            if let Some(vao) = self.vao {
                gl.bind_vertex_array(Some(vao));
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertices));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func_separate(glow::ZERO, glow::SRC_ALPHA, glow::ZERO, glow::SRC_ALPHA);
            gl.enable(glow::SCISSOR_TEST);
            let extent = (radius + 0.5).ceil() as i32;
            for x in [0, width as i32 - extent] {
                for y in [0, height as i32 - extent] {
                    gl.scissor(x, y, extent, extent);
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                }
            }
            if self.vao.is_some() {
                gl.bind_vertex_array(None);
            } else {
                gl.disable_vertex_attrib_array(0);
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
    }

    /// Called from App::on_exit while the native GL context is still current.
    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_buffer(self.vertices);
            if let Some(vao) = self.vao {
                gl.delete_vertex_array(vao);
            }
        }
    }
}

struct WindowShape {
    rect: Option<egui::Rect>,
    callback: Arc<egui_glow::CallbackFn>,
}

impl egui::Plugin for WindowShape {
    fn debug_name(&self) -> &'static str {
        "bmux window corners"
    }

    fn on_end_pass(&mut self, ctx: &egui::Context) {
        let square = ctx.input(|input| {
            let viewport = input.viewport();
            viewport.maximized.unwrap_or(false) || viewport.fullscreen.unwrap_or(false)
        });
        self.rect =
            (!square && ctx.viewport_id() == egui::ViewportId::ROOT).then(|| ctx.viewport_rect());
    }

    fn output_hook(&mut self, output: &mut egui::FullOutput) {
        if let Some(rect) = self.rect {
            // The output hook runs after all layers (including menus and debug
            // painting), and only once for the final result of a multipass frame.
            output.shapes.push(egui::epaint::ClippedShape {
                clip_rect: egui::Rect::EVERYTHING,
                shape: egui::PaintCallback {
                    rect,
                    callback: self.callback.clone(),
                }
                .into(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_mask_is_last_and_restores_after_maximizing_or_fullscreen() {
        let ctx = egui::Context::default();
        ctx.add_plugin(WindowShape {
            rect: None,
            callback: Arc::new(egui_glow::CallbackFn::new(|_, _| {})),
        });
        for (maximized, fullscreen) in [
            (false, false),
            (true, false),
            (false, false),
            (false, true),
            (false, false),
        ] {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(801.0, 601.0),
                )),
                ..Default::default()
            };
            let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
            viewport.maximized = Some(maximized);
            viewport.fullscreen = Some(fullscreen);
            let output = ctx.run(input, |ctx| {
                ctx.debug_painter()
                    .rect_filled(ctx.viewport_rect(), 0, egui::Color32::WHITE);
                if ctx.current_pass_index() == 0 {
                    ctx.request_discard("exercise multiple passes");
                }
            });
            let masks: Vec<_> = output
                .shapes
                .iter()
                .filter(|shape| matches!(shape.shape, egui::Shape::Callback(_)))
                .collect();
            assert_eq!(masks.len(), usize::from(!maximized && !fullscreen));
            if !maximized && !fullscreen {
                assert!(matches!(
                    output.shapes.last().unwrap().shape,
                    egui::Shape::Callback(_)
                ));
                assert_eq!(masks[0].clip_rect, egui::Rect::EVERYTHING);
            }
        }
    }
}
