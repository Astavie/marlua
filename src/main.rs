use std::{
    fs::read_to_string,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, Mutex,
    },
    thread,
};

use fastnes::{
    input::{self, Controllers},
    nes::NES,
    ppu::{DrawOptions, FastPPU},
};
use femtovg::{imgref::Img, renderer::OpenGl, rgb::RGBA8, Canvas, ImageFlags, Paint, Path};
use glutin::{
    config::ConfigTemplateBuilder,
    context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext},
    display::GetGlDisplay,
    prelude::{GlDisplay, NotCurrentGlContextSurfaceAccessor},
    surface::{GlSurface, Surface, SurfaceAttributesBuilder, WindowSurface},
};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasRawWindowHandle;
use rlua::{prelude::LuaError, Context, FromLua, Function, MultiValue, Scope};
use rlua::{Lua, StdLib};
use winit::{
    dpi::PhysicalSize,
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

struct Screen {
    el: EventLoop<()>,
    window: Window,
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    canvas: Canvas<OpenGl>,
}

impl Screen {
    fn new(title: &str, width: u32, height: u32) -> Self {
        // create window
        let el = EventLoop::new();
        let (window, config) = DisplayBuilder::new()
            .with_window_builder(Some(
                WindowBuilder::new()
                    .with_title(title)
                    .with_inner_size(PhysicalSize::new(width, height))
                    .with_resizable(false),
            ))
            .build(&el, ConfigTemplateBuilder::new(), |mut it| {
                it.next().unwrap()
            })
            .unwrap();

        // create surface
        let window = window.unwrap();
        let attrs = window.build_surface_attributes(SurfaceAttributesBuilder::new());

        let display = config.display();
        let surface = unsafe { display.create_window_surface(&config, &attrs).unwrap() };

        // create context
        let context = unsafe {
            display
                .create_context(
                    &config,
                    &ContextAttributesBuilder::new()
                        .with_context_api(ContextApi::OpenGl(None))
                        .build(Some(window.raw_window_handle())),
                )
                .unwrap()
                .make_current(&surface)
                .unwrap()
        };

        // create OpenGL
        let opengl = OpenGl::new_from_glutin_display(&display).unwrap();
        let mut canvas = Canvas::new(opengl).unwrap();
        canvas.set_size(width, height, 1.0);

        // return
        Self {
            el,
            window,
            surface,
            context,
            canvas,
        }
    }
    fn run(mut self, f: impl Fn(&mut Canvas<OpenGl>) + 'static) -> ! {
        self.el.run(move |event, _, cf| match event {
            // Window events
            winit::event::Event::WindowEvent {
                ref event,
                window_id,
            } if window_id == self.window.id() => match event {
                // Exit on window close
                winit::event::WindowEvent::CloseRequested => *cf = ControlFlow::Exit,
                _ => {}
            },

            // Redraw event
            winit::event::Event::MainEventsCleared => {
                f(&mut self.canvas);
                self.surface.swap_buffers(&self.context).unwrap();
            }

            _ => (),
        });
    }
}

unsafe fn as_rgba<const N: usize>(p: &[fastnes::ppu::Color; N]) -> &[RGBA8] {
    ::core::slice::from_raw_parts(
        (p as *const [fastnes::ppu::Color; N]) as *const RGBA8,
        ::core::mem::size_of::<[fastnes::ppu::Color; N]>(),
    )
}

fn run_lua<'lua>(ctx: Context<'lua>, frame: Arc<Frame>) -> Result<(), LuaError> {
    // create clock
    let mut clock = spin_sleep::LoopHelper::builder().build_with_target_rate(60);

    // create emulator
    let status = Arc::new(AtomicU8::new(0));
    let mut emulator = NES::read_ines(
        "rom/smb.nes",
        Controllers::standard(&status),
        FastPPU::new(),
    );

    // run nes to level 1-1
    for input in vec![
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0b00001000, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ] {
        status.store(input, Ordering::Relaxed);
        emulator.next_frame();
    }
    frame.update(&mut emulator);

    // run script
    ctx.scope(|scope| {
        let globals = ctx.globals();
        globals.set(
            "wait",
            scope.create_function_mut(|_, (time,): (u32,)| {
                for _ in 0..time {
                    clock.loop_start();
                    emulator.next_frame();
                    frame.update(&mut emulator);
                    clock.loop_sleep();
                }
                Ok(())
            })?,
        )?;

        globals.set(
            "toggle",
            scope.create_function(|ctx, buttons: MultiValue| {
                let mut input = status.load(Ordering::Relaxed);

                for button in buttons.into_iter().map(|v| String::from_lua(v, ctx)) {
                    let button = button?;
                    match button.to_uppercase().as_str() {
                        "A" | "JUMP" => {
                            input ^= 1 << 0;
                        }
                        "B" | "RUN" => {
                            input ^= 1 << 1;
                        }
                        "U" | "UP" => {
                            input ^= 1 << 4;
                            input &= !(1 << 5);
                        }
                        "D" | "DOWN" => {
                            input ^= 1 << 5;
                            input &= !(1 << 4);
                        }
                        "L" | "LEFT" => {
                            input ^= 1 << 6;
                            input &= !(1 << 7);
                        }
                        "R" | "RIGHT" => {
                            input ^= 1 << 7;
                            input &= !(1 << 6);
                        }
                        _ => todo!("give error for {}", button),
                    };
                }

                status.store(input, Ordering::Relaxed);
                Ok(())
            })?,
        )?;

        globals.set(
            "release",
            scope.create_function(|ctx, buttons: MultiValue| {
                let mut input = status.load(Ordering::Relaxed);

                for button in buttons.into_iter().map(|v| String::from_lua(v, ctx)) {
                    let button = button?;
                    match button.to_uppercase().as_str() {
                        "A" | "JUMP" => input &= !(1 << 0),
                        "B" | "RUN" => input &= !(1 << 1),
                        "U" | "UP" => input &= !(1 << 4),
                        "D" | "DOWN" => input &= !(1 << 5),
                        "L" | "LEFT" => input &= !(1 << 6),
                        "R" | "RIGHT" => input &= !(1 << 7),
                        _ => todo!("give error for {}", button),
                    };
                }

                status.store(input, Ordering::Relaxed);
                Ok(())
            })?,
        )?;

        globals.set(
            "press",
            scope.create_function(|ctx, buttons: MultiValue| {
                let mut input = status.load(Ordering::Relaxed);

                for button in buttons.into_iter().map(|v| String::from_lua(v, ctx)) {
                    let button = button?;
                    match button.to_uppercase().as_str() {
                        "A" | "JUMP" => {
                            input |= 1 << 0;
                        }
                        "B" | "RUN" => {
                            input |= 1 << 1;
                        }
                        "U" | "UP" => {
                            input |= 1 << 4;
                            input &= !(1 << 5);
                        }
                        "D" | "DOWN" => {
                            input |= 1 << 5;
                            input &= !(1 << 4);
                        }
                        "L" | "LEFT" => {
                            input |= 1 << 6;
                            input &= !(1 << 7);
                        }
                        "R" | "RIGHT" => {
                            input |= 1 << 7;
                            input &= !(1 << 6);
                        }
                        _ => todo!("give error for {}", button),
                    };
                }

                status.store(input, Ordering::Relaxed);
                Ok(())
            })?,
        )?;

        globals.set(
            "hold",
            scope.create_function(|ctx, input: MultiValue| {
                let mut buttons = input.into_vec();
                let time = buttons.pop();
                let buttons = MultiValue::from_vec(buttons);

                let globals = ctx.globals();
                let toggle: Function = globals.get("toggle")?;
                let wait: Function = globals.get("wait")?;

                toggle.call(buttons.clone())?;
                wait.call(time)?;
                toggle.call(buttons)?;

                Ok(())
            })?,
        )?;

        ctx.load(&read_to_string("script/mock.lua").unwrap())
            .exec()?;

        Ok(())
    })?;

    // run the rest of the emulator
    loop {
        clock.loop_start();
        emulator.next_frame();
        frame.update(&mut emulator);
        clock.loop_sleep();
    }
}

struct Frame {
    frame: Mutex<[fastnes::ppu::Color; 61440]>,
    ready: AtomicBool,
}

impl Frame {
    fn update(self: &Arc<Self>, emulator: &mut NES) {
        if self
            .ready
            .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            // program still needs to draw the current sent frame
            return;
        }

        let mut frame = self.frame.lock().unwrap();
        *frame = emulator.frame(DrawOptions::All);
    }
    fn frame(self: &Arc<Self>) -> [fastnes::ppu::Color; 61440] {
        self.ready.store(true, Ordering::Relaxed);
        self.frame.lock().unwrap().clone()
    }
}

fn main() -> Result<(), LuaError> {
    let frame = Arc::new(Frame {
        frame: Mutex::new(
            [fastnes::ppu::Color {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            }; 61440],
        ),
        ready: AtomicBool::new(true),
    });

    let clone = frame.clone();
    let handle = thread::spawn(move || {
        let lua = Lua::new_with(StdLib::all().difference(StdLib::OS | StdLib::IO | StdLib::DEBUG));
        lua.context(|ctx| run_lua(ctx, clone)).unwrap();
    });

    // open window
    Screen::new("Marlua", 640, 360).run(move |canvas| {
        let frame = frame.frame();

        // create image
        let img = Img::new(unsafe { as_rgba(&frame) }, 256, 240);
        let image = canvas.create_image(img, ImageFlags::NEAREST).unwrap();

        // draw image
        let fill_paint = Paint::image(image, 0.0, 0.0, 256.0, 240.0, 0.0, 1.0);
        let mut path = Path::new();
        path.rect(0.0, 0.0, 256.0, 240.0);
        canvas.fill_path(&mut path, &fill_paint);

        // destroy image
        // need to flush the canvas before being able to delete the image
        canvas.flush();
        canvas.delete_image(image);
    });
}
