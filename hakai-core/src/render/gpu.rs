//! The wgpu renderer — ported from `hakai/src/main.rs` (the Linux binary's Phase 4 draw
//! code), against wgpu 30 rather than 22. The *logic* is unchanged: a tiled damage layer
//! as GPU textures, an axis-aligned tile pipeline and a rotatable-sprite pipeline sharing
//! one shader module, procedural HUD/palette/credits pixmaps uploaded once, and one
//! render pass per frame in fixed z-bands (snapshot, tiles, particles, flames, flashes,
//! termites, cursor, HUD, palette, credits).
//!
//! wgpu 22 -> 30 differences applied throughout: `entry_point: Some(..)`;
//! `ImageCopyTexture`/`ImageDataLayout` -> `TexelCopyTextureInfo`/`TexelCopyBufferLayout`;
//! `multiview` -> `multiview_mask`; `push_constant_ranges` -> `immediate_size`;
//! `RenderPassColorAttachment.depth_slice`; `RenderPassDescriptor.multiview_mask`;
//! `SurfaceConfiguration.color_space`; `get_current_texture()` returns a
//! `CurrentSurfaceTexture` enum; `queue.present(frame)` not `frame.present()`.

use std::collections::HashMap;

use wgpu::util::DeviceExt;

use crate::icons::ToolIcons;
use crate::particles::ParticleKind;
use crate::tools::machine_gun::MachineGun;
use crate::tools::chain_saw::ChainSaw;
use crate::tools::flame_thrower::FlameThrower;
use crate::tools::hammer::Hammer;
use crate::tools::{Tool, ToolId};
use crate::DamageLayer;

use super::scene::GpuLayer;
use super::text::TextRenderer;
use super::theme;

// ── Per-tile GPU resources ──────────────────────────────────────────────────────────────

pub struct TileGpu {
    texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
}

/// Must match `shader.wgsl`'s `Tile` struct: two `vec2<f32>` back to back.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TileUniform {
    ndc_origin: [f32; 2],
    ndc_size: [f32; 2],
}

/// This tile's placement in clip space. The scene convention is y-down (matching input
/// and tiny-skia raster storage); wgpu NDC is y-up — so `ndc_size.y` comes out negative,
/// which is what makes the shader's unit quad land correctly without a flip inside it.
fn tile_ndc(origin_px: (f32, f32), size_px: (f32, f32), screen_px: (f32, f32)) -> TileUniform {
    TileUniform {
        ndc_origin: [-1.0 + 2.0 * origin_px.0 / screen_px.0, 1.0 - 2.0 * origin_px.1 / screen_px.1],
        ndc_size: [2.0 * size_px.0 / screen_px.0, -2.0 * size_px.1 / screen_px.1],
    }
}

fn create_tile_gpu(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    side_px: u32,
    ndc: TileUniform,
) -> TileGpu {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("damage-tile"),
        size: wgpu::Extent3d { width: side_px, height: side_px, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("tile-uniform"),
        contents: bytemuck::bytes_of(&ndc),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tile-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });

    TileGpu { texture, bind_group }
}

/// Uploads every dirty tile's pixels to its texture, then clears the dirty set.
fn upload_dirty_tiles(queue: &wgpu::Queue, gpu: &mut GpuLayer) {
    let Some(damage) = &gpu.damage else { return };
    let dirty: Vec<usize> = damage.dirty_indices().collect();
    for i in dirty {
        let (Some((pixels, w, h)), Some(tile)) = (damage.tile_pixels(i), gpu.tiles.get(i)) else {
            continue;
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tile.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }
    if let Some(damage) = &mut gpu.damage {
        damage.commit();
    }
}

// ── Cursor icons ─────────────────────────────────────────────────────────────────────────

pub struct IconGpu {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
}

fn create_icon_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    cursor_uniform: &wgpu::Buffer,
    pixmap: &tiny_skia::Pixmap,
) -> IconGpu {
    let (w, h) = (pixmap.width(), pixmap.height());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("icon"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        pixmap.data(),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("icon-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: cursor_uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });

    IconGpu { texture, bind_group }
}

fn create_sprite_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pixmap: &tiny_skia::Pixmap,
) -> (wgpu::Texture, wgpu::TextureView) {
    let (w, h) = (pixmap.width(), pixmap.height());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sprite"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        pixmap.data(),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[allow(dead_code)] // frozen-mode snapshot — wired to a capture producer in Phase 4
fn create_texture_from_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("snapshot"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        rgba,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * width), rows_per_image: Some(height) },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

// ── HUD elements ─────────────────────────────────────────────────────────────────────────

pub struct HudGpu {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    #[allow(dead_code)]
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    source: String,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

fn create_hud_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    pixmap: &tiny_skia::Pixmap,
    source: String,
) -> HudGpu {
    let (width, height) = (pixmap.width(), pixmap.height());
    let (texture, view) = create_sprite_texture(device, queue, pixmap);
    let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hud-uniform"),
        size: std::mem::size_of::<TileUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("hud-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    HudGpu { texture, view, width, height, source, uniform_buffer, bind_group }
}

fn create_hud_text(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    text: &mut TextRenderer,
    s: &str,
    size_px: f32,
    bold: bool,
    color: [u8; 4],
) -> Option<HudGpu> {
    let pixmap = text.rasterize(s, size_px, bold, color)?;
    Some(create_hud_gpu(device, queue, layout, sampler, &pixmap, s.to_string()))
}

#[allow(clippy::too_many_arguments)]
fn ensure_hud_text(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    text: &mut TextRenderer,
    cache: &mut Option<HudGpu>,
    s: &str,
    size_px: f32,
    bold: bool,
    color: [u8; 4],
) {
    if cache.as_ref().map(|c| c.source == s).unwrap_or(false) {
        return;
    }
    *cache = if s.is_empty() {
        None
    } else {
        create_hud_text(device, queue, layout, sampler, text, s, size_px, bold, color)
    };
}

fn draw_hud_element(
    queue: &wgpu::Queue,
    pass: &mut wgpu::RenderPass<'_>,
    element: &HudGpu,
    anchor_px: (f32, f32),
    anchor: (f32, f32),
    screen_px: (f32, f32),
) {
    let size_px = (element.width as f32, element.height as f32);
    let origin_px = (anchor_px.0 - anchor.0 * size_px.0, anchor_px.1 - anchor.1 * size_px.1);
    let ndc = tile_ndc(origin_px, size_px, screen_px);
    queue.write_buffer(&element.uniform_buffer, 0, bytemuck::bytes_of(&ndc));
    pass.set_bind_group(0, &element.bind_group, &[]);
    pass.draw(0..6, 0..1);
}

pub struct ToastGpu {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    source: String,
}

fn ensure_toast_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    text: &mut TextRenderer,
    cache: &mut Option<ToastGpu>,
    s: &str,
    size_px: f32,
    color: [u8; 4],
) {
    if cache.as_ref().map(|c| c.source == s).unwrap_or(false) {
        return;
    }
    *cache = if s.is_empty() {
        None
    } else {
        text.rasterize(s, size_px, false, color).map(|pixmap| {
            let (width, height) = (pixmap.width(), pixmap.height());
            let (texture, view) = create_sprite_texture(device, queue, &pixmap);
            ToastGpu { texture, view, width, height, source: s.to_string() }
        })
    };
}

const TERMITE_BASE_SIZE: (f32, f32) = (34.0, 19.0);
const FLAME_BASE_SIZE: (f32, f32) = (58.0, 100.0);
const FLAME_ANCHOR_TO_CENTER_Y: f32 = 0.40;

// ── HUD layout ───────────────────────────────────────────────────────────────────────────

const HUD_BAR_SIZE: (f32, f32) = (760.0, 34.0);
const HUD_BAR_BOTTOM_MARGIN: f32 = 40.0;
const HUD_BAR_PADDING: f32 = 16.0;
const HUD_LABEL_SIZE: f32 = 14.0;
const HUD_HINT_SIZE: f32 = 12.0;
const HUD_TOAST_BOTTOM_MARGIN: f32 = 92.0;
const HUD_TOAST_SIZE: f32 = 15.0;

const HUD_HINT_TEXT: &str = "1\u{2013}9 tool \u{b7} \u{2191}\u{2193} palette \u{b7} M mode \u{b7} C credits \u{b7} R clear \u{b7} Esc quit";

// ── Tool palette layout ──────────────────────────────────────────────────────────────────

const PALETTE_CELL: f32 = 66.0;
const PALETTE_GAP: f32 = 8.0;
const PALETTE_BOTTOM_MARGIN: f32 = 86.0;
const PALETTE_ICON_SIZE: f32 = PALETTE_CELL - 12.0;
const PALETTE_DIGIT_SIZE: f32 = 10.0;
const PALETTE_NAME_MARGIN: f32 = PALETTE_BOTTOM_MARGIN + PALETTE_CELL + 18.0;

fn palette_total_width() -> f32 {
    let count = ToolId::ALL.len() as f32;
    count * PALETTE_CELL + (count - 1.0) * PALETTE_GAP
}

fn palette_start_x(screen_width: f32) -> f32 {
    (screen_width - palette_total_width()) / 2.0
}

fn palette_cell_center(index: usize, screen: (f32, f32)) -> (f32, f32) {
    let x = palette_start_x(screen.0) + index as f32 * (PALETTE_CELL + PALETTE_GAP) + PALETTE_CELL / 2.0;
    let y = screen.1 - PALETTE_BOTTOM_MARGIN - PALETTE_CELL / 2.0;
    (x, y)
}

/// The tool under `point`, or `None` — the palette's hit-test. Half-gap margin so a click
/// between two cells still lands one of them.
pub fn palette_tool_at(point: (f32, f32), screen: (f32, f32)) -> Option<ToolId> {
    let half = PALETTE_CELL / 2.0 + PALETTE_GAP / 2.0;
    for (i, id) in ToolId::ALL.into_iter().enumerate() {
        let (cx, cy) = palette_cell_center(i, screen);
        if (point.0 - cx).abs() <= half && (point.1 - cy).abs() <= half {
            return Some(id);
        }
    }
    None
}

fn palette_icon_pixmap(icons: &mut ToolIcons, id: ToolId) -> &tiny_skia::Pixmap {
    match id {
        ToolId::Hammer => &icons.hammer().pixmap,
        ToolId::ChainSaw => &icons.chain_saw(false).pixmap,
        ToolId::MachineGun => &icons.machine_gun().pixmap,
        ToolId::FlameThrower => &icons.flame_thrower().pixmap,
        ToolId::ColorThrower => &icons.color_thrower().pixmap,
        ToolId::Phaser => &icons.phaser().pixmap,
        ToolId::Stamp => &icons.stamp(false).pixmap,
        ToolId::Termites => &icons.termite_hand().pixmap,
        ToolId::Washer => &icons.washer().pixmap,
    }
}

fn hud_rgba(color: (u8, u8, u8), alpha: u8) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(color.0, color.1, color.2, alpha)
}

fn hud_rgba_arr(color: (u8, u8, u8), alpha: u8) -> [u8; 4] {
    [color.0, color.1, color.2, alpha]
}

fn build_panel_pixmap(
    width_px: u32,
    height_px: u32,
    fill: tiny_skia::Color,
    stroke: Option<(tiny_skia::Color, f32)>,
) -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(width_px, height_px).expect("nonzero HUD panel size");
    let (w, h) = (width_px as f32, height_px as f32);

    let mut fill_paint = tiny_skia::Paint::default();
    fill_paint.set_color(fill);
    fill_paint.anti_alias = true;
    if let Some(rect) = tiny_skia::Rect::from_ltrb(0.0, 0.0, w, h) {
        let path = tiny_skia::PathBuilder::from_rect(rect);
        pixmap.fill_path(&path, &fill_paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
    }

    if let Some((color, width)) = stroke {
        let mut stroke_paint = tiny_skia::Paint::default();
        stroke_paint.set_color(color);
        stroke_paint.anti_alias = true;
        let stroke_style = tiny_skia::Stroke { width, ..Default::default() };
        let inset = width / 2.0;
        if let Some(rect) = tiny_skia::Rect::from_ltrb(inset, inset, w - inset, h - inset) {
            let path = tiny_skia::PathBuilder::from_rect(rect);
            pixmap.stroke_path(&path, &stroke_paint, &stroke_style, tiny_skia::Transform::identity(), None);
        }
    }

    pixmap
}

// ── Credits panel ────────────────────────────────────────────────────────────────────────

fn measure_char_width(text: &mut TextRenderer, size_px: f32) -> f32 {
    const REFERENCE: &str = "MMMMMMMMMM";
    text.rasterize(REFERENCE, size_px, false, [255, 255, 255, 255])
        .map(|p| p.width() as f32 / REFERENCE.chars().count() as f32)
        .unwrap_or(size_px * 0.6)
}

fn blit_over(dst: &mut tiny_skia::Pixmap, src: &tiny_skia::Pixmap, x: i32, y: i32) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let (sw, sh) = (src.width(), src.height());
    let dst_stride = dw as usize * 4;
    let src_stride = sw as usize * 4;
    let src_data = src.data();
    let dst_data = dst.data_mut();
    for sy in 0..sh {
        let dy = y + sy as i32;
        if dy < 0 || dy >= dh {
            continue;
        }
        for sx in 0..sw {
            let dx = x + sx as i32;
            if dx < 0 || dx >= dw {
                continue;
            }
            let si = sy as usize * src_stride + sx as usize * 4;
            let di = dy as usize * dst_stride + dx as usize * 4;
            let inv_a = 255 - src_data[si + 3] as u32;
            for c in 0..4 {
                let s = src_data[si + c] as u32;
                let d = dst_data[di + c] as u32;
                dst_data[di + c] = (s + (d * inv_a) / 255) as u8;
            }
        }
    }
}

const CREDITS_PADDING: f32 = 28.0;
const CREDITS_BODY_SIZE: f32 = 12.0;
const CREDITS_LINE_GAP: f32 = 5.0;

fn credits_text_color(color: crate::credits::TextColor, hud_colors: &theme::HudColors) -> [u8; 4] {
    match color {
        crate::credits::TextColor::White => hud_rgba_arr(hud_colors.foreground, 255),
        crate::credits::TextColor::Dim => hud_rgba_arr(hud_colors.foreground, 158),
        crate::credits::TextColor::Accent => hud_rgba_arr(hud_colors.accent, 255),
    }
}

fn build_credits_pixmap(text: &mut TextRenderer, screen_width: f32, hud_colors: &theme::HudColors) -> tiny_skia::Pixmap {
    let char_width = measure_char_width(text, CREDITS_BODY_SIZE);
    let max_width = 960.0_f32.min(screen_width - 120.0);
    let columns = (((max_width - CREDITS_PADDING * 2.0) / char_width).floor() as i64).max(40) as usize;

    let lines = crate::credits::build(columns);

    let width = columns as f32 * char_width + CREDITS_PADDING * 2.0;
    let mut height = CREDITS_PADDING * 2.0;
    for line in &lines {
        height += line.gap_before + line.size + CREDITS_LINE_GAP;
    }

    let mut pixmap = build_panel_pixmap(
        width.ceil().max(1.0) as u32,
        height.ceil().max(1.0) as u32,
        hud_rgba(hud_colors.background, 224),
        Some((hud_rgba(hud_colors.foreground, 71), 1.0)),
    );

    let mut y = CREDITS_PADDING;
    for line in &lines {
        y += line.gap_before + line.size;
        if !line.text.is_empty() {
            let color = credits_text_color(line.color, hud_colors);
            if let Some(glyph_pixmap) = text.rasterize(&line.text, line.size, false, color) {
                blit_over(&mut pixmap, &glyph_pixmap, CREDITS_PADDING as i32, (y - line.size) as i32);
            }
        }
        y += CREDITS_LINE_GAP;
    }

    pixmap
}

// ── Cursor / icon metadata ───────────────────────────────────────────────────────────────

fn icon_variant_key(active: ToolId, is_down: bool) -> &'static str {
    match active {
        ToolId::Hammer => "hammer",
        ToolId::ChainSaw => if is_down { "saw_cut" } else { "saw_idle" },
        ToolId::MachineGun => "machinegun",
        ToolId::FlameThrower => "flamethrower",
        ToolId::ColorThrower => "colorthrower",
        ToolId::Phaser => "phaser",
        ToolId::Stamp => if is_down { "stamp_down" } else { "stamp_up" },
        ToolId::Termites => "termites",
        ToolId::Washer => "washer",
    }
}

fn icon_metadata(icons: &mut ToolIcons, active: ToolId, is_down: bool) -> ((f32, f32), (f32, f32), (f32, f32)) {
    let icon = match active {
        ToolId::Hammer => icons.hammer(),
        ToolId::ChainSaw => icons.chain_saw(is_down),
        ToolId::MachineGun => icons.machine_gun(),
        ToolId::FlameThrower => icons.flame_thrower(),
        ToolId::ColorThrower => icons.color_thrower(),
        ToolId::Phaser => icons.phaser(),
        ToolId::Stamp => icons.stamp(is_down),
        ToolId::Termites => icons.termite_hand(),
        ToolId::Washer => icons.washer(),
    };
    (icon.hotspot, icon.pivot.unwrap_or(icon.hotspot), icon.point_size)
}

fn active_cursor_rotation(active: ToolId, tools: &HashMap<ToolId, Box<dyn Tool>>) -> f32 {
    match active {
        ToolId::Hammer => tools.get(&ToolId::Hammer).and_then(|t| t.as_any().downcast_ref::<Hammer>()).map(|h| h.cursor_rotation()).unwrap_or(0.0),
        ToolId::MachineGun => tools.get(&ToolId::MachineGun).and_then(|t| t.as_any().downcast_ref::<MachineGun>()).map(|g| g.cursor_rotation()).unwrap_or(0.0),
        ToolId::ChainSaw => tools.get(&ToolId::ChainSaw).and_then(|t| t.as_any().downcast_ref::<ChainSaw>()).map(|s| s.cursor_rotation()).unwrap_or(0.0),
        _ => 0.0,
    }
}

// ── Pipelines ────────────────────────────────────────────────────────────────────────────

pub struct Pipelines {
    pub tile: wgpu::RenderPipeline,
    pub tile_bind_group_layout: wgpu::BindGroupLayout,
    pub sprite: wgpu::RenderPipeline,
    pub sprite_bind_group_layout: wgpu::BindGroupLayout,
    pub sprite_additive: wgpu::RenderPipeline,
}

fn uniform_texture_sampler_layout(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ],
    })
}

pub fn create_pipelines(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipelines {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("hakai-shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
    });

    let tile_bind_group_layout = uniform_texture_sampler_layout(device, "tile-bind-group-layout");
    let sprite_bind_group_layout = uniform_texture_sampler_layout(device, "sprite-bind-group-layout");

    let tile_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("tile-pipeline-layout"),
        bind_group_layouts: &[Some(&tile_bind_group_layout)],
        immediate_size: 0,
    });
    let tile = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("tile-pipeline"),
        layout: Some(&tile_pipeline_layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let sprite_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sprite-pipeline-layout"),
        bind_group_layouts: &[Some(&sprite_bind_group_layout)],
        immediate_size: 0,
    });
    let sprite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sprite-pipeline"),
        layout: Some(&sprite_pipeline_layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_sprite"), buffers: &[], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_sprite"),
            targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let additive_blend = wgpu::BlendState {
        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
    };
    let sprite_additive = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sprite-additive-pipeline"),
        layout: Some(&sprite_pipeline_layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_sprite"), buffers: &[], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_sprite"),
            targets: &[Some(wgpu::ColorTargetState { format, blend: Some(additive_blend), write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    Pipelines { tile, tile_bind_group_layout, sprite, sprite_bind_group_layout, sprite_additive }
}

// ── Rotated sprites ──────────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RotatedSpriteUniform {
    top_left: [f32; 2],
    top_right: [f32; 2],
    bottom_left: [f32; 2],
    bottom_right: [f32; 2],
    alpha: f32,
    _pad: f32,
}

fn rotated_sprite_ndc(center_px: (f32, f32), size_px: (f32, f32), rotation: f32, screen_px: (f32, f32), alpha: f32) -> RotatedSpriteUniform {
    let (hw, hh) = (size_px.0 / 2.0, size_px.1 / 2.0);
    let (cos_r, sin_r) = (rotation.cos(), rotation.sin());
    let to_ndc = |local: (f32, f32)| -> [f32; 2] {
        let rotated = (local.0 * cos_r - local.1 * sin_r, local.0 * sin_r + local.1 * cos_r);
        let world_px = (center_px.0 + rotated.0, center_px.1 + rotated.1);
        [-1.0 + 2.0 * world_px.0 / screen_px.0, 1.0 - 2.0 * world_px.1 / screen_px.1]
    };
    RotatedSpriteUniform {
        top_left: to_ndc((-hw, -hh)),
        top_right: to_ndc((hw, -hh)),
        bottom_left: to_ndc((-hw, hh)),
        bottom_right: to_ndc((hw, hh)),
        alpha,
        _pad: 0.0,
    }
}

fn rotated_sprite_ndc_pivot(pivot_world: (f32, f32), pivot_local: (f32, f32), size_px: (f32, f32), rotation: f32, screen_px: (f32, f32), alpha: f32) -> RotatedSpriteUniform {
    let (cos_r, sin_r) = (rotation.cos(), rotation.sin());
    let to_ndc = |corner_local: (f32, f32)| -> [f32; 2] {
        let rel = (corner_local.0 - pivot_local.0, corner_local.1 - pivot_local.1);
        let rotated = (rel.0 * cos_r - rel.1 * sin_r, rel.0 * sin_r + rel.1 * cos_r);
        let world_px = (pivot_world.0 + rotated.0, pivot_world.1 + rotated.1);
        [-1.0 + 2.0 * world_px.0 / screen_px.0, 1.0 - 2.0 * world_px.1 / screen_px.1]
    };
    RotatedSpriteUniform {
        top_left: to_ndc((0.0, 0.0)),
        top_right: to_ndc((size_px.0, 0.0)),
        bottom_left: to_ndc((0.0, size_px.1)),
        bottom_right: to_ndc((size_px.0, size_px.1)),
        alpha,
        _pad: 0.0,
    }
}

fn draw_rotated_sprite(
    device: &wgpu::Device,
    pass: &mut wgpu::RenderPass<'_>,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    texture_view: &wgpu::TextureView,
    ndc: &RotatedSpriteUniform,
    label: &str,
) {
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::bytes_of(ndc),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(texture_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..6, 0..1);
}

// ── The shared, non-per-output GPU resources ─────────────────────────────────────────────

/// Everything the renderer needs that isn't per-output: the device/queue, the pipelines,
/// and every procedurally-built texture cache. Built once by `Assets::build`.
pub struct Assets {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub pipelines: Pipelines,
    pub sampler: wgpu::Sampler,

    icon_gpu: HashMap<&'static str, IconGpu>,
    cursor_uniform_buffer: wgpu::Buffer,
    termite_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    shell_texture: (wgpu::Texture, wgpu::TextureView),
    droplet_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    flame_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    sliver_textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    flash_texture: (wgpu::Texture, wgpu::TextureView),

    hud_panel: HudGpu,
    hud_hint: Option<HudGpu>,
    hud_label: Option<HudGpu>,
    palette_panel: (wgpu::Texture, wgpu::TextureView, f32, f32),
    palette_cell_normal: (wgpu::Texture, wgpu::TextureView),
    palette_cell_selected: (wgpu::Texture, wgpu::TextureView),
    palette_icons: HashMap<ToolId, (wgpu::Texture, wgpu::TextureView)>,
    palette_digits: HashMap<ToolId, (wgpu::Texture, wgpu::TextureView, f32, f32)>,
    credits_panel: (wgpu::Texture, wgpu::TextureView, f32, f32),

    pub hud_colors: theme::HudColors,
}

impl Assets {
    /// Builds the pipelines and every static texture cache. `screen_width` sizes the
    /// credits panel (built once, like the Linux binary does from the first output).
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        icons: &mut ToolIcons,
        sprites: &mut crate::sprites::SpriteFactory,
        decals: &mut crate::DecalFactory,
        text: &mut TextRenderer,
        hud_colors: theme::HudColors,
        screen_width: f32,
    ) -> Self {
        let pipelines = create_pipelines(&device, format);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tile-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let cursor_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cursor-uniform"),
            size: std::mem::size_of::<RotatedSpriteUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = &pipelines.sprite_bind_group_layout;
        let mut icon_gpu = HashMap::new();
        icon_gpu.insert("hammer", create_icon_gpu(&device, &queue, layout, &sampler, &cursor_uniform_buffer, &icons.hammer().pixmap));
        icon_gpu.insert("saw_idle", create_icon_gpu(&device, &queue, layout, &sampler, &cursor_uniform_buffer, &icons.chain_saw(false).pixmap));
        icon_gpu.insert("saw_cut", create_icon_gpu(&device, &queue, layout, &sampler, &cursor_uniform_buffer, &icons.chain_saw(true).pixmap));
        icon_gpu.insert("machinegun", create_icon_gpu(&device, &queue, layout, &sampler, &cursor_uniform_buffer, &icons.machine_gun().pixmap));
        icon_gpu.insert("flamethrower", create_icon_gpu(&device, &queue, layout, &sampler, &cursor_uniform_buffer, &icons.flame_thrower().pixmap));
        icon_gpu.insert("colorthrower", create_icon_gpu(&device, &queue, layout, &sampler, &cursor_uniform_buffer, &icons.color_thrower().pixmap));
        icon_gpu.insert("phaser", create_icon_gpu(&device, &queue, layout, &sampler, &cursor_uniform_buffer, &icons.phaser().pixmap));
        icon_gpu.insert("stamp_up", create_icon_gpu(&device, &queue, layout, &sampler, &cursor_uniform_buffer, &icons.stamp(false).pixmap));
        icon_gpu.insert("stamp_down", create_icon_gpu(&device, &queue, layout, &sampler, &cursor_uniform_buffer, &icons.stamp(true).pixmap));
        icon_gpu.insert("termites", create_icon_gpu(&device, &queue, layout, &sampler, &cursor_uniform_buffer, &icons.termite_hand().pixmap));
        icon_gpu.insert("washer", create_icon_gpu(&device, &queue, layout, &sampler, &cursor_uniform_buffer, &icons.washer().pixmap));

        let termite_textures = vec![
            create_sprite_texture(&device, &queue, sprites.termite(0)),
            create_sprite_texture(&device, &queue, sprites.termite(1)),
        ];
        let shell_texture = create_sprite_texture(&device, &queue, sprites.shell());
        let paint_colors = decals.paint_colors();
        let droplet_textures: Vec<_> = (0..crate::DecalFactory::DEFAULT_PAINT_COLORS.len() as i64)
            .map(|i| create_sprite_texture(&device, &queue, sprites.droplet(i, &paint_colors)))
            .collect();
        let flame_textures: Vec<_> = (0..crate::sprites::SpriteFactory::FLAME_FRAMES)
            .map(|f| create_sprite_texture(&device, &queue, sprites.standing_flame(f)))
            .collect();
        let sliver_textures: Vec<_> = (0..crate::DecalFactory::SLIVER_VARIANTS)
            .map(|v| create_sprite_texture(&device, &queue, decals.sliver(v)))
            .collect();
        let flash_texture = create_sprite_texture(&device, &queue, sprites.flash());

        let hud_panel_pixmap = build_panel_pixmap(
            HUD_BAR_SIZE.0 as u32,
            HUD_BAR_SIZE.1 as u32,
            hud_rgba(hud_colors.background, 158),
            Some((hud_rgba(hud_colors.foreground, 56), 1.0)),
        );
        let hud_panel = create_hud_gpu(&device, &queue, &pipelines.tile_bind_group_layout, &sampler, &hud_panel_pixmap, String::new());
        let hud_hint = create_hud_text(&device, &queue, &pipelines.tile_bind_group_layout, &sampler, text, HUD_HINT_TEXT, HUD_HINT_SIZE, false, hud_rgba_arr(hud_colors.foreground, 153));
        let initial_label = format!("{} \u{b7} {}", ToolId::Hammer.key_digit(), ToolId::Hammer.display_name());
        let hud_label = create_hud_text(&device, &queue, &pipelines.tile_bind_group_layout, &sampler, text, &initial_label, HUD_LABEL_SIZE, false, hud_rgba_arr(hud_colors.foreground, 255));

        let palette_panel_pixmap = build_panel_pixmap(
            (palette_total_width() + 24.0) as u32,
            (PALETTE_CELL + 24.0) as u32,
            hud_rgba(hud_colors.background, 168),
            Some((hud_rgba(hud_colors.foreground, 56), 1.0)),
        );
        let (ppt, ppv) = create_sprite_texture(&device, &queue, &palette_panel_pixmap);
        let palette_panel = (ppt, ppv, palette_panel_pixmap.width() as f32, palette_panel_pixmap.height() as f32);

        let cell_normal_pixmap = build_panel_pixmap(PALETTE_CELL as u32, PALETTE_CELL as u32, hud_rgba(hud_colors.foreground, 18), None);
        let palette_cell_normal = create_sprite_texture(&device, &queue, &cell_normal_pixmap);
        let cell_selected_pixmap = build_panel_pixmap(
            PALETTE_CELL as u32,
            PALETTE_CELL as u32,
            hud_rgba(hud_colors.accent, 66),
            Some((hud_rgba(hud_colors.accent, 191), 1.5)),
        );
        let palette_cell_selected = create_sprite_texture(&device, &queue, &cell_selected_pixmap);

        let mut palette_icons = HashMap::new();
        let mut palette_digits = HashMap::new();
        for id in ToolId::ALL {
            palette_icons.insert(id, create_sprite_texture(&device, &queue, palette_icon_pixmap(icons, id)));
            let digit = format!("{}", id.key_digit());
            if let Some(pixmap) = text.rasterize(&digit, PALETTE_DIGIT_SIZE, true, hud_rgba_arr(hud_colors.foreground, 140)) {
                let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
                let (tex, view) = create_sprite_texture(&device, &queue, &pixmap);
                palette_digits.insert(id, (tex, view, w, h));
            }
        }

        let credits_pixmap = build_credits_pixmap(text, screen_width, &hud_colors);
        let (cw, ch) = (credits_pixmap.width() as f32, credits_pixmap.height() as f32);
        let (ct, cv) = create_sprite_texture(&device, &queue, &credits_pixmap);
        let credits_panel = (ct, cv, cw, ch);

        Self {
            device,
            queue,
            pipelines,
            sampler,
            icon_gpu,
            cursor_uniform_buffer,
            termite_textures,
            shell_texture,
            droplet_textures,
            flame_textures,
            sliver_textures,
            flash_texture,
            hud_panel,
            hud_hint,
            hud_label,
            palette_panel,
            palette_cell_normal,
            palette_cell_selected,
            palette_icons,
            palette_digits,
            credits_panel,
            hud_colors,
        }
    }

    /// Rebuilds `hud_label` when the active tool changes — `select_tool`'s tail.
    pub fn refresh_tool_label(&mut self, text: &mut TextRenderer, id: ToolId) {
        let s = format!("{} \u{b7} {}", id.key_digit(), id.display_name());
        ensure_hud_text(
            &self.device,
            &self.queue,
            &self.pipelines.tile_bind_group_layout,
            &self.sampler,
            text,
            &mut self.hud_label,
            &s,
            HUD_LABEL_SIZE,
            false,
            hud_rgba_arr(self.hud_colors.foreground, 255),
        );
    }

    /// (Re)builds a `GpuLayer`'s damage layer and its GPU tiles at the given point size.
    pub fn build_damage(&self, gpu: &mut GpuLayer) {
        let mut damage = DamageLayer::new(gpu.width as f32, gpu.height as f32, gpu.scale);
        damage.mark_all_dirty();

        let (cols, rows) = damage.grid_size();
        let side_px = (DamageLayer::TILE_SIDE * damage.scale()) as u32;
        let mut tiles = Vec::with_capacity(cols * rows);
        for i in 0..(cols * rows) {
            let origin = damage.tile_origin(i);
            let ndc = tile_ndc(origin, (DamageLayer::TILE_SIDE, DamageLayer::TILE_SIDE), (gpu.width as f32, gpu.height as f32));
            tiles.push(create_tile_gpu(&self.device, &self.pipelines.tile_bind_group_layout, &self.sampler, side_px, ndc));
        }

        gpu.damage = Some(damage);
        gpu.tiles = tiles;
    }
}

// ── The frame ────────────────────────────────────────────────────────────────────────────

/// Renders one output's frame into its own surface. Draw order and z-bands are the Linux
/// binary's `State::render`, unchanged.
pub fn render(a: &Assets, text: &mut TextRenderer, icons: &mut ToolIcons, show_cursor: bool, gpu: &mut GpuLayer) {
    let device = &a.device;
    let queue = &a.queue;
    let pipeline = &a.pipelines.tile;
    let sprite_pipeline = &a.pipelines.sprite;
    let sprite_additive_pipeline = &a.pipelines.sprite_additive;
    let sprite_bind_group_layout = &a.pipelines.sprite_bind_group_layout;
    let sampler = &a.sampler;

    upload_dirty_tiles(queue, gpu);

    let variant_key = icon_variant_key(gpu.active_tool, gpu.is_down);
    if show_cursor && a.icon_gpu.contains_key(variant_key) {
        let (hotspot, pivot, point_size) = icon_metadata(icons, gpu.active_tool, gpu.is_down);
        let rotation = active_cursor_rotation(gpu.active_tool, &gpu.tools);
        let pivot_world = (gpu.mouse.0 + (pivot.0 - hotspot.0) * point_size.0, gpu.mouse.1 + (pivot.1 - hotspot.1) * point_size.1);
        let pivot_local = (pivot.0 * point_size.0, pivot.1 * point_size.1);
        let ndc = rotated_sprite_ndc_pivot(pivot_world, pivot_local, point_size, rotation, (gpu.width as f32, gpu.height as f32), 1.0);
        queue.write_buffer(&a.cursor_uniform_buffer, 0, bytemuck::bytes_of(&ndc));
    }

    use wgpu::CurrentSurfaceTexture as C;
    let frame = match gpu.wgpu_surface.get_current_texture() {
        C::Success(f) | C::Suboptimal(f) => f,
        C::Outdated | C::Lost => {
            log::warn!("surface Outdated/Lost — reconfiguring");
            gpu.reconfigure_surface(device);
            return;
        }
        other => {
            log::warn!("get_current_texture: {other:?}");
            return;
        }
    };
    let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hakai-frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        let screen_px = (gpu.width as f32, gpu.height as f32);

        if let Some((_, view, width, height)) = &gpu.snapshot_texture {
            pass.set_pipeline(sprite_pipeline);
            let center = (screen_px.0 / 2.0, screen_px.1 / 2.0);
            let ndc = rotated_sprite_ndc(center, (*width as f32, *height as f32), 0.0, screen_px, 1.0);
            draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, view, &ndc, "snapshot");
        }

        pass.set_pipeline(pipeline);
        for tile in &gpu.tiles {
            pass.set_bind_group(0, &tile.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        pass.set_pipeline(sprite_pipeline);
        for particle in gpu.particles.iter() {
            let tex_view = match particle.kind {
                ParticleKind::Shell => Some(&a.shell_texture.1),
                ParticleKind::Droplet { color_index } => {
                    let i = (color_index.rem_euclid(a.droplet_textures.len().max(1) as i64)) as usize;
                    a.droplet_textures.get(i).map(|(_, v)| v)
                }
                ParticleKind::Generic { variant } => {
                    let i = (variant.rem_euclid(a.sliver_textures.len().max(1) as i64)) as usize;
                    a.sliver_textures.get(i).map(|(_, v)| v)
                }
            };
            let Some(tex_view) = tex_view else { continue };
            let ndc = rotated_sprite_ndc(particle.position, particle.size, particle.rotation, screen_px, 1.0);
            draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, tex_view, &ndc, "particle");
        }

        if let Some(flame_thrower) = gpu.tools.get(&ToolId::FlameThrower).and_then(|t| t.as_any().downcast_ref::<FlameThrower>()) {
            pass.set_pipeline(sprite_additive_pipeline);
            for flame in flame_thrower.flames() {
                if a.flame_textures.is_empty() {
                    break;
                }
                let frame = ((flame.age / 0.07) as i64).rem_euclid(a.flame_textures.len() as i64) as usize;
                let Some((_, tex_view)) = a.flame_textures.get(frame) else { continue };
                let size_px = (FLAME_BASE_SIZE.0 * flame.scale, FLAME_BASE_SIZE.1 * flame.scale);
                let center = (flame.position.0, flame.position.1 - FLAME_ANCHOR_TO_CENTER_Y * size_px.1);
                let alpha = if flame.life_fraction > 0.8 { ((1.0 - flame.life_fraction) / 0.2).max(0.0) } else { 1.0 };
                let ndc = rotated_sprite_ndc(center, size_px, 0.0, screen_px, alpha);
                draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, tex_view, &ndc, "flame");
            }
        }

        if let Some(gun) = gpu.tools.get(&ToolId::MachineGun).and_then(|t| t.as_any().downcast_ref::<MachineGun>()) {
            pass.set_pipeline(sprite_additive_pipeline);
            for flash in gun.flashes() {
                let side = flash.size * (1.0 + 0.5 * flash.life_fraction);
                let alpha = (1.0 - flash.life_fraction).max(0.0);
                let ndc = rotated_sprite_ndc(flash.position, (side, side), flash.rotation, screen_px, alpha);
                draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, &a.flash_texture.1, &ndc, "flash");
            }
        }

        pass.set_pipeline(sprite_pipeline);
        for termite in gpu.termites.iter() {
            let frame = (termite.frame.max(0) as usize) % a.termite_textures.len().max(1);
            let Some((_, tex_view)) = a.termite_textures.get(frame) else { continue };
            let size_px = (TERMITE_BASE_SIZE.0 * termite.scale, TERMITE_BASE_SIZE.1 * termite.scale);
            let rotation = termite.heading + std::f32::consts::PI;
            let ndc = rotated_sprite_ndc(termite.position, size_px, rotation, screen_px, 1.0);
            draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, tex_view, &ndc, "termite");
        }

        if show_cursor {
            if let Some(icon) = a.icon_gpu.get(variant_key) {
                pass.set_pipeline(sprite_pipeline);
                pass.set_bind_group(0, &icon.bind_group, &[]);
                pass.draw(0..6, 0..1);
            }
        }

        pass.set_pipeline(pipeline);
        let bar_center = (screen_px.0 / 2.0, screen_px.1 - HUD_BAR_BOTTOM_MARGIN);
        draw_hud_element(queue, &mut pass, &a.hud_panel, bar_center, (0.5, 0.5), screen_px);
        if let Some(label) = &a.hud_label {
            let anchor = (bar_center.0 - HUD_BAR_SIZE.0 / 2.0 + HUD_BAR_PADDING, bar_center.1);
            draw_hud_element(queue, &mut pass, label, anchor, (0.0, 0.5), screen_px);
        }
        if let Some(hint) = &a.hud_hint {
            let anchor = (bar_center.0 + HUD_BAR_SIZE.0 / 2.0 - HUD_BAR_PADDING, bar_center.1);
            draw_hud_element(queue, &mut pass, hint, anchor, (1.0, 0.5), screen_px);
        }

        if let Some((toast_text, alpha)) = gpu.hud.toast() {
            let toast_text = toast_text.to_string();
            ensure_toast_gpu(device, queue, text, &mut gpu.toast_gpu, &toast_text, HUD_TOAST_SIZE, hud_rgba_arr(a.hud_colors.foreground, 255));
            if let Some(toast) = &gpu.toast_gpu {
                pass.set_pipeline(sprite_pipeline);
                let center = (screen_px.0 / 2.0, screen_px.1 - HUD_TOAST_BOTTOM_MARGIN);
                let ndc = rotated_sprite_ndc(center, (toast.width as f32, toast.height as f32), 0.0, screen_px, alpha);
                draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, &toast.view, &ndc, "toast");
            }
        } else if gpu.toast_gpu.is_some() {
            gpu.toast_gpu = None;
        }

        let palette_alpha = gpu.hud.palette_alpha();
        if palette_alpha > 0.0 {
            pass.set_pipeline(sprite_pipeline);
            let (_, view, pw, ph) = &a.palette_panel;
            let center = (screen_px.0 / 2.0, screen_px.1 - PALETTE_BOTTOM_MARGIN - PALETTE_CELL / 2.0);
            let ndc = rotated_sprite_ndc(center, (*pw, *ph), 0.0, screen_px, palette_alpha);
            draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, view, &ndc, "palette-panel");

            for (i, id) in ToolId::ALL.into_iter().enumerate() {
                let center = palette_cell_center(i, screen_px);
                let cell_bg = if id == gpu.active_tool { &a.palette_cell_selected } else { &a.palette_cell_normal };
                let ndc = rotated_sprite_ndc(center, (PALETTE_CELL, PALETTE_CELL), 0.0, screen_px, palette_alpha);
                draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, &cell_bg.1, &ndc, "palette-cell");

                if let Some((_, view)) = a.palette_icons.get(&id) {
                    let icon_center = (center.0, center.1 - 4.0);
                    let ndc = rotated_sprite_ndc(icon_center, (PALETTE_ICON_SIZE, PALETTE_ICON_SIZE), 0.0, screen_px, palette_alpha);
                    draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, view, &ndc, "palette-icon");
                }
                if let Some((_, view, dw, dh)) = a.palette_digits.get(&id) {
                    let digit_center = (center.0, center.1 + PALETTE_CELL / 2.0 - 8.0);
                    let ndc = rotated_sprite_ndc(digit_center, (*dw, *dh), 0.0, screen_px, palette_alpha);
                    draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, view, &ndc, "palette-digit");
                }
            }
            if let Some(label) = &a.hud_label {
                let center = (screen_px.0 / 2.0, screen_px.1 - PALETTE_NAME_MARGIN);
                let ndc = rotated_sprite_ndc(center, (label.width as f32, label.height as f32), 0.0, screen_px, palette_alpha);
                draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, &label.view, &ndc, "palette-name");
            }
        }

        let credits_alpha = gpu.hud.credits_alpha();
        if credits_alpha > 0.0 {
            let (_, view, w, h) = &a.credits_panel;
            pass.set_pipeline(sprite_pipeline);
            let center = (screen_px.0 / 2.0, screen_px.1 / 2.0);
            let ndc = rotated_sprite_ndc(center, (*w, *h), 0.0, screen_px, credits_alpha);
            draw_rotated_sprite(device, &mut pass, sprite_bind_group_layout, sampler, view, &ndc, "credits-panel");
        }
    }
    queue.submit([encoder.finish()]);
    queue.present(frame);
}

/// Frozen mode's captured-snapshot upload — `capture.rs`'s `to_rgba` output, straight to a
/// texture. Kept here so `create_texture_from_rgba` stays private to the renderer.
#[allow(dead_code)] // Phase 4
pub fn upload_snapshot(a: &Assets, rgba: &[u8], width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView, u32, u32) {
    let (t, v) = create_texture_from_rgba(&a.device, &a.queue, rgba, width, height);
    (t, v, width, height)
}
