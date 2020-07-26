/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

/*! This is a player for WebGPU traces.
!*/

use player::{gfx_select, GlobalPlay as _, IdentityPassThroughFactory};
use wgc::device::trace;

use std::{
    fs,
    path::{Path, PathBuf},
};

fn main() {
    #[cfg(feature = "winit")]
    use winit::{event_loop::EventLoop, window::WindowBuilder};

    wgc::logging::subscriber::initialize_default_subscriber(
        std::env::var("WGPU_CHROME_TRACE")
            .as_ref()
            .map(Path::new)
            .ok(),
    );

    #[cfg(feature = "renderdoc")]
    #[cfg_attr(feature = "winit", allow(unused))]
    let mut rd = renderdoc::RenderDoc::<renderdoc::V110>::new()
        .expect("Failed to connect to RenderDoc: are you running without it?");

    //TODO: setting for the backend bits
    //TODO: setting for the target frame, or controls

    let dir = match std::env::args().nth(1) {
        Some(arg) if Path::new(&arg).is_dir() => PathBuf::from(arg),
        _ => panic!("Provide the dir path as the parameter"),
    };

    log::info!("Loading trace '{:?}'", dir);
    let file = fs::File::open(dir.join(trace::FILE_NAME)).unwrap();
    let mut actions: Vec<trace::Action> = ron::de::from_reader(file).unwrap();
    actions.reverse(); // allows us to pop from the top
    log::info!("Found {} actions", actions.len());

    #[cfg(feature = "winit")]
    let event_loop = {
        log::info!("Creating a window");
        EventLoop::new()
    };
    #[cfg(feature = "winit")]
    let window = WindowBuilder::new()
        .with_title("wgpu player")
        .with_resizable(false)
        .build(&event_loop)
        .unwrap();

    let global =
        wgc::hub::Global::new("player", IdentityPassThroughFactory, wgt::BackendBit::all());
    let mut command_buffer_id_manager = wgc::hub::IdentityManager::default();

    #[cfg(feature = "winit")]
    let surface =
        global.instance_create_surface(&window, wgc::id::TypedId::zip(0, 1, wgt::Backend::Empty));

    let device = match actions.pop() {
        Some(trace::Action::Init { desc, backend }) => {
            log::info!("Initializing the device for backend: {:?}", backend);
            let adapter = global
                .pick_adapter(
                    &wgc::instance::RequestAdapterOptions {
                        power_preference: wgt::PowerPreference::Default,
                        #[cfg(feature = "winit")]
                        compatible_surface: Some(surface),
                        #[cfg(not(feature = "winit"))]
                        compatible_surface: None,
                    },
                    wgc::instance::AdapterInputs::IdSet(
                        &[wgc::id::TypedId::zip(0, 0, backend)],
                        |id| id.backend(),
                    ),
                )
                .expect("Unable to find an adapter for selected backend");

            let info = gfx_select!(adapter => global.adapter_get_info(adapter));
            log::info!("Picked '{}'", info.name);
            gfx_select!(adapter => global.adapter_request_device(
                adapter,
                &desc,
                None,
                wgc::id::TypedId::zip(1, 0, wgt::Backend::Empty)
            ))
            .expect("Failed to request device")
        }
        _ => panic!("Expected Action::Init"),
    };
    log::info!("Executing actions");
    #[cfg(not(feature = "winit"))]
    {
        #[cfg(feature = "renderdoc")]
        rd.start_frame_capture(std::ptr::null(), std::ptr::null());

        while let Some(action) = actions.pop() {
            gfx_select!(device => global.process(device, action, &dir, &mut command_buffer_id_manager));
        }

        #[cfg(feature = "renderdoc")]
        rd.end_frame_capture(std::ptr::null(), std::ptr::null());
        gfx_select!(device => global.device_poll(device, true)).unwrap();
    }
    #[cfg(feature = "winit")]
    {
        use winit::{
            event::{ElementState, Event, KeyboardInput, VirtualKeyCode, WindowEvent},
            event_loop::ControlFlow,
        };

        let mut frame_count = 0;
        event_loop.run(move |event, _, control_flow| {
            *control_flow = ControlFlow::Poll;
            match event {
                Event::MainEventsCleared => {
                    window.request_redraw();
                }
                Event::RedrawRequested(_) => loop {
                    match actions.pop() {
                        Some(trace::Action::CreateSwapChain(id, desc)) => {
                            log::info!("Initializing the swapchain");
                            assert_eq!(id.to_surface_id(), surface);
                            window.set_inner_size(winit::dpi::PhysicalSize::new(
                                desc.width,
                                desc.height,
                            ));
                            gfx_select!(device => global.device_create_swap_chain(device, surface, &desc));
                        }
                        Some(trace::Action::PresentSwapChain(id)) => {
                            frame_count += 1;
                            log::debug!("Presenting frame {}", frame_count);
                            gfx_select!(device => global.swap_chain_present(id));
                            break;
                        }
                        Some(action) => {
                            gfx_select!(device => global.process(device, action, &dir, &mut command_buffer_id_manager));
                        }
                        None => break,
                    }
                },
                Event::WindowEvent { event, .. } => match event {
                    WindowEvent::KeyboardInput {
                        input:
                            KeyboardInput {
                                virtual_keycode: Some(VirtualKeyCode::Escape),
                                state: ElementState::Pressed,
                                ..
                            },
                        ..
                    }
                    | WindowEvent::CloseRequested => {
                        *control_flow = ControlFlow::Exit;
                    }
                    _ => {}
                },
                Event::LoopDestroyed => {
                    log::info!("Closing");
                    gfx_select!(device => global.device_poll(device, true));
                }
                _ => {}
            }
        });
    }
}
