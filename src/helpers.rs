//! Helper functions
use conrod::backend::glium::glium;
use conrod::glium::glutin::EventsLoop;
use conrod::glium::Display;
use conrod::Ui;
use find_folder;
use image::imageops::FilterType;
use image::DynamicImage;

/// Load the given `dyn_image` as a [`glium Texture2d`](glium::texture::Texture2d) struct.
pub fn load_img(display: &glium::Display, dyn_img: DynamicImage) -> glium::texture::Texture2d {
    let rgba_image = dyn_img.to_rgba8();
    let image_dimensions = rgba_image.dimensions();
    let raw_image = glium::texture::RawImage2d::from_raw_rgba_reversed(
        &rgba_image.into_raw(),
        image_dimensions,
    );
    let texture = glium::texture::Texture2d::new(display, raw_image).unwrap();
    texture
}

/// Load the fonts for this ui.
///
/// Fonts are located in the assets folder.
pub fn load_fonts(ui: &mut Ui) {
    let assets = find_folder::Search::KidsThenParents(3, 5)
        .for_folder("assets")
        .unwrap();
    let font_path = assets.join("fonts/NotoSans/NotoSans-Regular.ttf");
    ui.fonts.insert_from_file(font_path).unwrap();
}

/// Load the "image-not-found" png to use when artwork can't be found.
///
/// Located in the assets folder.
pub fn load_img_not_found() -> DynamicImage {
    let assets = find_folder::Search::ParentsThenKids(3, 3)
        .for_folder("assets")
        .unwrap();
    let path = assets.join("images/image-not-found.png");
    let img = image::open(&std::path::Path::new(&path)).unwrap();
    img.resize(500, 220, FilterType::Lanczos3)
}

/// Load the "placeholder" png to use when artwork can't be loaded just yet.
///
/// Located in the assets folder.
pub fn load_placeholder_img() -> DynamicImage {
    let assets = find_folder::Search::ParentsThenKids(3, 3)
        .for_folder("assets")
        .unwrap();
    let path = assets.join("images/placeholder.png");
    let img = image::open(&std::path::Path::new(&path)).unwrap();
    img.resize(500, 220, FilterType::Lanczos3)
}

/// Build the [`glium Display`](Display) and [`EventsLoop`] for the window.
pub fn build_display() -> (Display, EventsLoop, Ui) {
    let events_loop = glium::glutin::EventsLoop::new();
    let window = glium::glutin::WindowBuilder::new()
        .with_title("Hello +")
        .with_dimensions(crate::DISPLAY_WIDTH, crate::DISPLAY_HEIGHT);
    let context = glium::glutin::ContextBuilder::new()
        .with_vsync(true)
        .with_multisampling(4);

    let mut ui =
        conrod::UiBuilder::new([crate::DISPLAY_WIDTH as f64, crate::DISPLAY_HEIGHT as f64]).build();
    load_fonts(&mut ui);
    (
        glium::Display::new(window, context, &events_loop).unwrap(),
        events_loop,
        ui,
    )
}
