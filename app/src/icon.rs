pub fn rgba(size: usize) -> Vec<u8> {
    // Intentamos cargar la imagen desde los assets
    let img_bytes = include_bytes!("../assets/icon.png");
    if let Ok(img) = image::load_from_memory(img_bytes) {
        let resized = img.resize_exact(
            size as u32,
            size as u32,
            image::imageops::FilterType::Lanczos3,
        );
        let rgba = resized.to_rgba8();
        return rgba.into_raw();
    }

    // Fallback: si falla, dibuja un cuadrado gris para que no se rompa la app
    let mut pixels = vec![255u8; size * size * 4];
    for i in 0..size * size {
        pixels[i * 4] = 0x2C; // R
        pixels[i * 4 + 1] = 0x2C; // G
        pixels[i * 4 + 2] = 0x2E; // B
        pixels[i * 4 + 3] = 255; // A
    }
    pixels
}
