#[macro_use]
extern crate conrod;
use api::{Api, SetData};
use conrod::backend::glium::glium::backend::glutin::glutin::VirtualKeyCode;
use conrod::backend::glium::glium::{self, Surface};
use conrod::glium::glutin::EventsLoop;
use conrod::glium::Display;
use conrod::image::Map;
use conrod::{widget, Colorable, Positionable, Sizeable, Ui, Widget};
use find_folder;
use image::DynamicImage;
use log::{debug, info, warn};
use std::time::Instant;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

const NAVIGATION_KEYS_DEBOUNCE_THRESHOLD: u128 = 150;

struct LeftRight(usize);
struct TopDown(usize);

widget_ids!(struct Ids { text, img, img2 });

pub struct EventLoop {
    ui_needs_update: bool,
    last_update: std::time::Instant,
}

impl EventLoop {
    pub fn new() -> Self {
        EventLoop {
            last_update: std::time::Instant::now(),
            ui_needs_update: true,
        }
    }

    /// Produce an iterator yielding all available events.
    pub fn next(
        &mut self,
        events_loop: &mut glium::glutin::EventsLoop,
    ) -> Vec<glium::glutin::Event> {
        // We don't want to loop any faster than 60 FPS, so wait until it has been at least 16ms
        // since the last yield.
        let last_update = self.last_update;
        let sixteen_ms = std::time::Duration::from_millis(16);
        let duration_since_last_update = std::time::Instant::now().duration_since(last_update);
        if duration_since_last_update < sixteen_ms {
            std::thread::sleep(sixteen_ms - duration_since_last_update);
        }

        // Collect all pending events.
        let mut events = Vec::new();
        events_loop.poll_events(|event| events.push(event));

        // If there are no events and the UI does not need updating, wait
        // for the next event.
        if events.is_empty() && !self.ui_needs_update {
            events_loop.run_forever(|event| {
                events.push(event);
                glium::glutin::ControlFlow::Break
            });
        }

        self.ui_needs_update = false;
        self.last_update = std::time::Instant::now();

        events
    }

    /// Notifies the event loop that the `Ui` requires another update whether
    /// or not there are any pending events.
    ///
    /// This is primarily used on the occasion that some part of the UI is
    /// still animating and requires further updates to do so.
    pub fn needs_update(&mut self) {
        self.ui_needs_update = true;
    }
}

fn load_img(display: &glium::Display, dyn_img: DynamicImage) -> glium::texture::Texture2d {
    let rgba_image = dyn_img.to_rgba8();
    let image_dimensions = rgba_image.dimensions();
    let raw_image = glium::texture::RawImage2d::from_raw_rgba_reversed(
        &rgba_image.into_raw(),
        image_dimensions,
    );
    let texture = glium::texture::Texture2d::new(display, raw_image).unwrap();
    texture
}

fn load_img_not_found() -> DynamicImage {
    let assets = find_folder::Search::ParentsThenKids(3, 3)
        .for_folder("assets")
        .unwrap();
    let path = assets.join("images/image-not-found.png");
    image::open(&std::path::Path::new(&path)).unwrap()
}

fn build_display() -> (Display, EventsLoop) {
    let events_loop = glium::glutin::EventsLoop::new();
    let window = glium::glutin::WindowBuilder::new()
        .with_title("Hello Plus")
        .with_dimensions(WIDTH, HEIGHT);
    let context = glium::glutin::ContextBuilder::new()
        .with_vsync(true)
        .with_multisampling(4);
    (
        glium::Display::new(window, context, &events_loop).unwrap(),
        events_loop,
    )
}

fn load_fonts(ui: &mut Ui) {
    let assets = find_folder::Search::KidsThenParents(3, 5)
        .for_folder("assets")
        .unwrap();
    let font_path = assets.join("fonts/NotoSans/NotoSans-Regular.ttf");
    ui.fonts.insert_from_file(font_path).unwrap();
}

struct SetRow<'a> {
    title: &'a str,
    set_data: SetData<'a>,
    set_idx: usize,
}

impl<'a> SetRow<'a> {
    fn new(set_data: SetData<'a>, set_idx: usize) -> Self {
        debug!("Initialized Set row: {:?}", set_data);
        let title = set_data.get_title();
        Self {
            set_data,
            title,
            set_idx,
        }
    }

    fn get_top_offset(&self) -> f64 {
        (self.set_idx as f64) * 200.0 + 70.0
    }

    fn show(
        &self,
        display: &Display,
        ui: &mut Ui,
        image_map: &mut Map<glium::texture::Texture2d>,
        ids: &Ids,
        item_idx: usize,
    ) {
        let ui = &mut ui.set_widgets();
        let img = self
            .set_data
            .get_home_tile_image(item_idx)
            .unwrap_or_else(|e| {
                warn!("{}", e);
                load_img_not_found()
            });

        let img = load_img(display, img);
        let (w, h) = (img.get_width(), img.get_height().unwrap());
        let img = image_map.insert(img);

        widget::Image::new(img)
            .w_h(w as f64, h as f64)
            .top_left_with_margins_on(ui.window, self.get_top_offset(), 20.0)
            .set(ids.img, ui);

        widget::Text::new(self.title)
            .up_from(ids.img, 14.0)
            .color(conrod::color::WHITE)
            .font_size(36)
            .set(ids.text, ui);
    }
}

struct DisplayController<'a> {
    rows: Vec<SetRow<'a>>,
    display: &'a Display,
    image_map: Map<glium::texture::Texture2d>,
    api_handle: &'a Api,
    ids: Ids,
}

impl<'a> DisplayController<'a> {
    fn new(display: &'a Display, api_handle: &'a Api, ui: &mut Ui) -> Self {
        let ids = Ids::new(ui.widget_id_generator());
        Self {
            rows: Vec::new(),
            display,
            image_map: Map::<glium::texture::Texture2d>::new(),
            api_handle,
            ids,
        }
    }

    fn show_n_rows(&mut self, n: usize, ui: &mut Ui) {
        for set_idx in 0..n {
            let row_data = self.api_handle.get_set(set_idx).expect("TODO testing");
            let set_row = SetRow::new(row_data, set_idx);
            set_row.show(self.display, ui, &mut self.image_map, &self.ids, 0);
            self.rows.push(set_row);
        }
    }

    fn navigate_to(&mut self, set_idx: usize, item_idx: usize, ui: &mut Ui) {
        info!("Image map size {}", self.image_map.len());
        if set_idx >= self.rows.len() {
            let row_data = self.api_handle.get_set(set_idx).expect("TODO testing");
            let set_row = SetRow::new(row_data, set_idx);
            self.rows.push(set_row);
        }
        self.rows[set_idx].show(self.display, ui, &mut self.image_map, &self.ids, item_idx);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let (display, mut events_loop) = build_display();

    let mut ui = conrod::UiBuilder::new([WIDTH as f64, HEIGHT as f64]).build();
    load_fonts(&mut ui);

    let api_handle = {
        let mut a = api::Api::new();
        a.load_home_data()?;
        a
    };

    let mut renderer = conrod::backend::glium::Renderer::new(&display).unwrap();
    let mut event_loop = EventLoop::new();

    let mut controller = DisplayController::new(&display, &api_handle, &mut ui);
    controller.show_n_rows(4, &mut ui);

    let mut item_idx = LeftRight(0);
    let mut set_idx = TopDown(0);
    let mut navigation_debounce = Instant::now();

    'main: loop {
        // Render the `Ui` and then display it on the screen.
        if let Some(primitives) = ui.draw_if_changed() {
            renderer.fill(&display, primitives, &controller.image_map);
            let mut target = display.draw();
            target.clear_color(0.0, 0.0, 0.013, 1.0);
            renderer
                .draw(&display, &mut target, &controller.image_map)
                .unwrap();
            target.finish().unwrap();
        }
        let mut events = Vec::new();
        events_loop.poll_events(|event| events.push(event));

        for event in event_loop.next(&mut events_loop) {
            match event {
                glium::glutin::Event::WindowEvent { event, .. } => match event {
                    glium::glutin::WindowEvent::Closed
                    | glium::glutin::WindowEvent::KeyboardInput {
                        input:
                            glium::glutin::KeyboardInput {
                                virtual_keycode: Some(glium::glutin::VirtualKeyCode::Escape),
                                ..
                            },
                        ..
                    } => break 'main,
                    glium::glutin::WindowEvent::KeyboardInput {
                        input:
                            glium::glutin::KeyboardInput {
                                virtual_keycode: Some(key_code),
                                ..
                            },
                        ..
                    } => {
                        if user_navigated(
                            key_code,
                            &mut item_idx,
                            &mut set_idx,
                            &mut navigation_debounce,
                        ) {
                            controller.navigate_to(set_idx.0, item_idx.0, &mut ui);
                        }
                    }
                    _ => (),
                },
                _ => (),
            }
        }
    }
    Ok(())
}

fn user_navigated(
    key_code: VirtualKeyCode,
    item: &mut LeftRight,
    set_num: &mut TopDown,
    navigation_debounce: &mut Instant,
) -> bool {
    if (*navigation_debounce).elapsed().as_millis() < NAVIGATION_KEYS_DEBOUNCE_THRESHOLD {
        return false;
    }
    *navigation_debounce = Instant::now();
    if key_code == VirtualKeyCode::Left {
        if (*item).0 != 0 {
            (*item).0 -= 1;
        }
        return true;
    } else if key_code == VirtualKeyCode::Right {
        (*item).0 += 1;
        return true;
    } else if key_code == VirtualKeyCode::Up {
        if (*set_num).0 != 0 {
            (*set_num).0 -= 1;
        }
        return true;
    } else if key_code == VirtualKeyCode::Down {
        (*set_num).0 += 1;
        return true;
    }

    return false;
}
