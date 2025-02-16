use std::sync::Arc;
use tauri::{Emitter, Manager, Runtime, State};
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowDisplayAffinity,
    WINDOW_DISPLAY_AFFINITY,
};
use windows::Win32::Foundation::HWND;
use win_desktop_duplication::{devices::*, tex_reader::*, co_init, set_process_dpi_awareness, DesktopDuplicationApi, DuplicationApiOptions};
use parking_lot::RwLock;
use win_desktop_duplication::errors::DDApiError;

#[derive(Default, Clone)]
pub struct FrameBuffer {
    data: Vec<u8>,
    width: u32,
    height: u32,
    fps: u32
}

#[derive(Default, Clone)]
pub struct CaptureState {
    frame_buffer: Arc<RwLock<FrameBuffer>>,
}

impl CaptureState {
    fn new() -> Self {
        Self {
            frame_buffer: Arc::new(RwLock::new(FrameBuffer::default())),
        }
    }
}

fn enable_capture_protection<R: Runtime>(window: &tauri::Window<R>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let hwnd = HWND(window.hwnd().unwrap().0);
        let affinity = WINDOW_DISPLAY_AFFINITY(0x00000011);
        unsafe {
            SetWindowDisplayAffinity(hwnd, affinity).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn configure_window<R: Runtime>(window: &tauri::Window<R>) -> Result<(), String> {
    // Enable capture protection
    enable_capture_protection(window)?;
    Ok(())
}


fn process_image(
    original: &[u8],
    orig_width: u32,
    crop_x: u32,
    crop_y: u32,
    crop_width: u32,
    crop_height: u32,
    scale_factor: u32
) -> Vec<u8> {
    let new_width = crop_width / scale_factor;
    let new_height = crop_height / scale_factor;

    let mut downsampled = Vec::with_capacity((new_width * new_height * 4) as usize);

    let x_scale = (crop_width as f32 / new_width as f32).ceil() as u32;
    let y_scale = (crop_height as f32 / new_height as f32).ceil() as u32;

    for y in 0..new_height {
        for x in 0..new_width {
            // Calculate the source pixel in the original image based on the crop and scale factor
            let src_x = crop_x + x * x_scale;
            let src_y = crop_y + y * y_scale;

            if src_x < crop_x + crop_width && src_y < crop_y + crop_height {
                let src_idx = ((src_y * orig_width + src_x) * 4) as usize;
                if src_idx + 3 < original.len() {
                    // Fix colour channels
                    downsampled.push(original[src_idx + 2]); // R => B
                    downsampled.push(original[src_idx + 1]); // G => G
                    downsampled.push(original[src_idx]);     // B => R
                    downsampled.push(original[src_idx + 3]); // A => A
                }
            }
        }
    }

    downsampled
}


async fn start_capture(window: tauri::Window, frame_buffer: Arc<RwLock<FrameBuffer>>) -> Result<(), String> {
    set_process_dpi_awareness();
    co_init();

    let mut adapters = AdapterFactory::new();
    let adapter = adapters.find(|adapter| {
        adapter.get_display_by_idx(0).is_some()
    })
        .ok_or("No suitable display adapters found")?;

    let output = adapter.get_display_by_idx(0)
        .ok_or("No displays found for the selected adapter")?;

    // Configure for fastest frame acquisition
    let mut dupl = DesktopDuplicationApi::new(adapter, output.clone())
        .map_err(|e| format!("Failed to initialize desktop duplication: {:?}", e))?;

    let mut options = DuplicationApiOptions::default();
    options.skip_cursor = true;
    dupl.configure(options);

    let (device, ctx) = dupl.get_device_and_ctx();
    let mut texture_reader = TextureReader::new(device, ctx);

    let mut frame_counter = 0u32;
    let mut fps_counter = 0u32;
    let mut last_second = std::time::Instant::now();

    loop {
        // Use acquire_next_frame_now for immediate frame capture
        match dupl.acquire_next_frame_now() {
            Ok(tex) => {
                let desc = tex.desc();

                let scale_factor = 4;

                // Prepare a new buffer for the frame
                let mut frame_data = Vec::with_capacity((desc.width * desc.height * 4) as usize);

                // Read texture data using texture_reader
                match texture_reader.get_data(&mut frame_data, &tex) {
                    Ok(_) => {
                        // Get window position and size directly
                        let window_pos = window.outer_position().map_err(|e| e.to_string())?;
                        let window_size = window.outer_size().map_err(|e| e.to_string())?;

                        // Crop the image to the window's area
                        let processed_data = process_image(
                            &frame_data,
                            desc.width,
                            window_pos.x as u32,
                            window_pos.y as u32,
                            window_size.width,
                            window_size.height,
                            scale_factor
                        );

                        // Notify frontend about new frame
                        frame_counter = frame_counter.wrapping_add(1);
                        fps_counter += 1;

                        // Update the shared state
                        {
                            let current_time = std::time::Instant::now();
                            let mut buffer = frame_buffer.write();
                            buffer.data = processed_data;
                            buffer.width = window_size.width as u32 / scale_factor;
                            buffer.height = window_size.height as u32 / scale_factor;
                            if current_time.duration_since(last_second).as_secs() >= 1 {
                                // println!("FPS: {}", fps_counter);
                                buffer.fps = fps_counter.clone();
                                fps_counter = 0;
                                last_second = current_time;
                            }
                        }

                        if let Err(e) = window.emit("frame-ready", frame_counter) {
                            eprintln!("Failed to emit frame-ready event: {:?}", e);
                        }
                    },
                    Err(e) => {
                        eprintln!("Failed to get texture data: {:?}", e);
                        continue;
                    }
                }
            },
            Err(e) => {
                eprintln!("Failed to acquire frame: {:?}", e);
                // Handle potential recovery scenarios
                if matches!(e, DDApiError::AccessLost | DDApiError::AccessDenied) {
                    // Potentially reinitialize duplication API
                    break;
                }
            }
        }
    }

    Ok(())
}

#[tauri::command]
fn get_frame_data(state: State<'_, CaptureState>) -> Result<(Vec<u8>, u32, u32, u32), String> {
    let frame_buffer = state.frame_buffer.read();

    if !frame_buffer.data.is_empty() && frame_buffer.width > 0 && frame_buffer.height > 0 {
        Ok((
            frame_buffer.data.clone(),
            frame_buffer.width,
            frame_buffer.height,
            frame_buffer.fps
        ))
    } else {
        Err("No valid frame data available.".to_string())
    }
}


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(CaptureState::new())
        .setup(|app| {
            let capture_state = app.state::<CaptureState>();

            if let Some(window) = app.get_window("main") {
                if let Err(e) = configure_window(&window) {
                    eprintln!("Failed to configure window: {}", e);
                }

                let state_clone = capture_state.frame_buffer.clone();

                tauri::async_runtime::spawn(async move {
                    if let Err(e) = start_capture(window, state_clone).await {
                        eprintln!("Capture error: {}", e);
                    }
                });
            }
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_frame_data])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
