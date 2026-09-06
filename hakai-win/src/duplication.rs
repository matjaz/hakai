//! DXGI Desktop Duplication — the Windows producer for `capture::BrightnessMap`.
//!
//! Replaces the Linux binary's `zwlr_screencopy_v1` dance. The consumer is unchanged:
//! `BrightnessMap::update(bytes, width, height, stride)` and `capture::to_rgba` both take
//! exactly a strided BGRA buffer, which is what a mapped D3D11 staging texture is.
//!
//! **Graceful degradation is the point.** Duplication genuinely fails — unsupported on
//! some hybrid-graphics setups, `DXGI_ERROR_ACCESS_LOST` on every resolution change /
//! secure-desktop transition / session switch, and not available at all inside many
//! Remote Desktop sessions. Every one of those paths returns `None`/reinitialises rather
//! than panicking; the app then falls back to a random impact-sound variant, exactly as
//! it does on a compositor without the Wayland protocol.

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource, DXGI_ERROR_ACCESS_LOST,
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};

pub struct DesktopDuplication {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    output: IDXGIOutput1,
    dupl: IDXGIOutputDuplication,
    /// A CPU-readable copy target, (re)created to match the incoming frame's size.
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    /// `AcquireNextFrame` succeeded and `ReleaseFrame` hasn't run yet — the API requires
    /// exactly one release per successful acquire before the next acquire.
    holding: bool,
}

impl DesktopDuplication {
    /// `None` on any failure — a laptop without a duplication-capable output, a Remote
    /// Desktop session, another process already duplicating the primary output, ...
    pub fn new() -> Option<Self> {
        unsafe {
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .inspect_err(|e| log::warn!("D3D11CreateDevice failed ({e}) — impact sounds will use a random variant"))
            .ok()?;
            let device = device?;
            let context = context?;

            let dxgi_device: IDXGIDevice = device.cast().ok()?;
            let adapter = dxgi_device.GetAdapter().ok()?;
            let output = adapter
                .EnumOutputs(0)
                .inspect_err(|e| log::warn!("IDXGIAdapter::EnumOutputs(0) failed ({e})"))
                .ok()?;
            let output: IDXGIOutput1 = output.cast().ok()?;

            let dupl = output
                .DuplicateOutput(&device)
                .inspect_err(|e| log::warn!("IDXGIOutput1::DuplicateOutput failed ({e}) — brightness capture disabled"))
                .ok()?;

            log::info!("desktop duplication: active");
            Some(Self { device, context, output, dupl, staging: None, holding: false })
        }
    }

    /// Re-runs `DuplicateOutput` after `DXGI_ERROR_ACCESS_LOST`. Keeps the same device/
    /// output; only the duplication object is stale.
    fn recreate(&mut self) -> bool {
        unsafe {
            if self.holding {
                let _ = self.dupl.ReleaseFrame();
                self.holding = false;
            }
            match self.output.DuplicateOutput(&self.device) {
                Ok(dupl) => {
                    self.dupl = dupl;
                    self.staging = None;
                    log::info!("desktop duplication: re-acquired after ACCESS_LOST");
                    true
                }
                Err(e) => {
                    log::warn!("desktop duplication: re-acquire failed ({e})");
                    false
                }
            }
        }
    }

    /// Grabs the most recent desktop frame and hands its mapped BGRA pixels to `f` as
    /// `(bytes, width, height, stride)`. `None` when there's no new frame yet
    /// (`WAIT_TIMEOUT`, the common case at this cadence), or on any recoverable error.
    pub fn capture<R>(&mut self, f: impl FnOnce(&[u8], u32, u32, u32) -> R) -> Option<R> {
        unsafe {
            // Defensive: a previous call that returned early mid-frame.
            if self.holding {
                let _ = self.dupl.ReleaseFrame();
                self.holding = false;
            }

            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            // 0 ms — non-blocking; we're called from the frame loop and just want
            // "whatever's ready."
            match self.dupl.AcquireNextFrame(0, &mut info, &mut resource) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return None,
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    self.recreate();
                    return None;
                }
                Err(e) => {
                    log::debug!("AcquireNextFrame: {e}");
                    return None;
                }
            }
            self.holding = true;

            let result = (|| {
                let resource = resource?;
                let frame: ID3D11Texture2D = resource.cast().ok()?;
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                frame.GetDesc(&mut desc);
                let (w, h) = (desc.Width, desc.Height);

                // (Re)make the staging texture if the frame size changed.
                let need_new = !matches!(self.staging, Some((_, sw, sh)) if sw == w && sh == h);
                if need_new {
                    let sdesc = D3D11_TEXTURE2D_DESC {
                        Width: w,
                        Height: h,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                        Usage: D3D11_USAGE_STAGING,
                        BindFlags: 0,
                        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                        MiscFlags: 0,
                    };
                    let mut tex: Option<ID3D11Texture2D> = None;
                    self.device.CreateTexture2D(&sdesc, None, Some(&mut tex)).ok()?;
                    self.staging = Some((tex?, w, h));
                }
                let staging = &self.staging.as_ref()?.0;

                self.context.CopyResource(staging, &frame);

                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                self.context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).ok()?;
                let bytes = std::slice::from_raw_parts(mapped.pData as *const u8, (mapped.RowPitch * h) as usize);
                let out = f(bytes, w, h, mapped.RowPitch);
                self.context.Unmap(staging, 0);
                Some(out)
            })();

            let _ = self.dupl.ReleaseFrame();
            self.holding = false;
            result
        }
    }
}

impl Drop for DesktopDuplication {
    fn drop(&mut self) {
        if self.holding {
            unsafe {
                let _ = self.dupl.ReleaseFrame();
            }
        }
    }
}
