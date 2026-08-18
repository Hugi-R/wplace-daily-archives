use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, RequestMode, Response};

use wimage::{PalettedImage};
use wimage::tilehistory;
use wimage::tilehistory::{TileHistory, DateHours};
use wimage::palette;

pub fn init_img(x1: i64, y1: i64, x2: i64, y2: i64, background: u8) -> PalettedImage {
    assert!(x2 >= x1 && y2 >= y1);

    let height = ((y2+1)-y1)*1000;
    let width = ((x2+1)-x1)*1000;
    assert!((height*width) < (30000*30000)); // That's already 900MB of indices! Also few things will display a bigger image.
    PalettedImage { width: width as usize, height: height as usize, indices: vec![background; (width*height) as usize] }
}

pub fn copy_img(src: &PalettedImage, dst: &mut PalettedImage, tile_x_offset: i64, tile_y_offset: i64) {
    let offset_x = (tile_x_offset * 1000) as usize;
    let offset_y = (tile_y_offset * 1000) as usize;
    for y in 0..src.height {
        let src_row_start = y * src.width;
        let dst_row_start = (y + offset_y) * dst.width + offset_x;
        
        dst.indices[dst_row_start..dst_row_start + src.width]
            .copy_from_slice(&src.indices[src_row_start..src_row_start + src.width]);
    }
}

#[wasm_bindgen]
extern "C" {
    // Use `js_namespace` here to bind `console.log(..)` instead of just
    // `log(..)`
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);

    #[wasm_bindgen]
    async fn log_user_message(key: &str, args: Vec<String>);
}

macro_rules! console_log {
    // Note that this is using the `log` function imported above during
    // `bare_bones`
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}

#[wasm_bindgen]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub async fn wasm_screenshot(base_url: &str, z: i64, version: u32, x1: i64, y1: i64, x2: i64, y2: i64) -> Result<Vec<u8>, JsValue> {
    let mut target = init_img(x1, y1, x2, y2, palette::TRANSPARENT);
    let version = DateHours(version);

    let opts = RequestInit::new();
    opts.set_method("GET");
    opts.set_mode(RequestMode::Cors);

    let window = web_sys::window().unwrap();

    let total_tiles = (x2-x1+1) * (y2-y1+1);
    log_user_message("downloading_tiles", vec![total_tiles.to_string()]).await;

    for y in y1..(y2+1) {
        for x in x1..(x2+1) {
            let url = format!("{}/{}/{}/{}/{}.zst", base_url, version.week(), z, x, y);
            log_user_message("downloading_url", vec![url.clone()]).await;
            let request = Request::new_with_str_and_init(&url, &opts)?;
            let resp_value = JsFuture::from(window.fetch_with_request(&request)).await?;
            assert!(resp_value.is_instance_of::<Response>());
            let resp: Response = resp_value.dyn_into().unwrap();
            let img = match resp.status() {
                200 => {
                    use js_sys::Uint8Array;
                    let jsvalue = JsFuture::from(resp.array_buffer()?).await?;
                    let data = Uint8Array::new(&jsvalue).to_vec();
                    if data.len() == 0 {
                        PalettedImage { width: 1000, height: 1000, indices: vec![0u8; 1000*1000] }
                    } else {
                        let tiles = TileHistory::from_bytes(&data)
                            .map_err(|e| wasm_bindgen::JsValue::from_str(&format!("Failed to decode tile history: {}", e)))?;
                        match tiles.get(version) {
                            Ok(Some(img)) => img,
                            Ok(None) => PalettedImage { width: 1000, height: 1000, indices: vec![0u8; 1000*1000] },
                            Err(e) => {
                                return Err(wasm_bindgen::JsValue::from_str(&format!("Failed to reconstruct image: {}", e)));
                            }
                        }
                    }
                },
                404 => {
                    PalettedImage { width: 1000, height: 1000, indices: vec![0u8; 1000*1000] }
                },
                s => return Err(format!("Unexpected status code: {}", s).into())
            };
            assert!(img.height == 1000 && img.width == 1000);
            copy_img(&img, &mut target, x-x1, y-y1);
        }
    }


    log_user_message("download_finish", vec![]).await;
    log_user_message("creating_image", vec![]).await;
    let png = target.to_png().map_err(|e| wasm_bindgen::JsValue::from_str(&format!("Failed to encode PNG: {}", e)))?;
    log_user_message("done", vec![]).await;
    Ok(png)
}

#[wasm_bindgen]
pub async fn wasm_video(base_url: &str, z: i64, x1: i64, y1: i64, x2: i64, y2: i64, from: u32, to: u32) -> Result<Vec<u8>, JsValue> {
    use std::collections::HashMap;
    
    let opts = RequestInit::new();
    opts.set_method("GET");
    opts.set_mode(RequestMode::Cors);

    let window = web_sys::window().unwrap();

    let total_tiles = (x2-x1+1) * (y2-y1+1);
    log_user_message("downloading_tiles", vec![total_tiles.to_string()]).await;

    let mut history:HashMap<(u16, u16), TileHistory> = HashMap::new();
    for y in y1..(y2+1) {
        for x in x1..(x2+1) {
            let url = format!("{}/all/{}/{}/{}.zst?from={}&to={}", base_url, z, x, y, from, to);
            log_user_message("downloading_url", vec![url.clone()]).await;
            let request = Request::new_with_str_and_init(&url, &opts)?;
            let resp_value = JsFuture::from(window.fetch_with_request(&request)).await?;
            assert!(resp_value.is_instance_of::<Response>());
            let resp: Response = resp_value.dyn_into().unwrap();
            match resp.status() {
                200 => {
                    use js_sys::Uint8Array;
                    let jsvalue = JsFuture::from(resp.array_buffer()?).await?;
                    let data = Uint8Array::new(&jsvalue).to_vec();
                    if data.len() > 0 {
                        let th = TileHistory::from_bytes(&data)
                            .map_err(|e| wasm_bindgen::JsValue::from_str(&format!("Failed to decode tile history: {}", e)))?;
                        history.insert((x as u16, y as u16), th);
                    }
                },
                404 => {
                    // empty tile, do nothing
                },
                s => return Err(format!("Unexpected status code: {}", s).into())
            }
        }
    }

    log_user_message("download_finish", vec![]).await;
    log_user_message("creating_video", vec![]).await;
    let png = tilehistory::apng_from_history(history, 200)
        .map_err(|e| wasm_bindgen::JsValue::from_str(&format!("Failed to create APNG: {}", e)))?;
    log_user_message("done", vec![]).await;
    Ok(png)
}

#[wasm_bindgen]
pub fn get_image(version: u32, data: &[u8]) -> Result<Vec<u8>, wasm_bindgen::JsValue> {
    if data.len() == 0 {
        return Err(wasm_bindgen::JsValue::from_str("Empty data"));
    }
    let version = DateHours(version);

    let tiles = TileHistory::from_bytes(&data)
        .map_err(|e| wasm_bindgen::JsValue::from_str(&format!("Failed to decode tile history: {}", e)))?;

    let base_paletted = tiles.get(version)
        .map_err(|e| wasm_bindgen::JsValue::from_str(&format!("Failed to reconstruct image: {}", e)))?;
    let base_paletted = base_paletted.ok_or(wasm_bindgen::JsValue::from_str(&format!("No image found for version {}", version.0)))?;
    base_paletted.to_png().map_err(|e| wasm_bindgen::JsValue::from_str(&format!("Failed to encode png: {}", e)))
}