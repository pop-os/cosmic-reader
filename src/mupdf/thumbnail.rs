use std::{error::Error, path::Path};
use url::Url;

pub fn main(
    input: &Url,
    output: &Path,
    size_opt: Option<(u32, u32)>,
) -> Result<(), Box<dyn Error>> {
    let path = input
        .to_file_path()
        .map_err(|()| format!("{:?} is not a path", input))?;
    let doc = mupdf::Document::open(path.as_os_str())?;
    let page = doc.load_page(0)?;
    let display_list = page.to_display_list(false)?;

    let scale = match size_opt {
        Some((width, height)) => {
            let bounds = page.bounds()?;
            ((width as f32) / bounds.width()).min((height as f32) / bounds.height())
        }
        //TODO: correct default scale?
        None => 1.0,
    };

    let matrix = mupdf::Matrix::new_scale(scale, scale);
    let pixmap = display_list.to_pixmap(&matrix, &mupdf::Colorspace::device_rgb(), false)?;
    let output_str = output
        .to_str()
        .ok_or_else(|| format!("{:?} is not valid UTF-8", output))?;
    pixmap.save_as(output_str, mupdf::ImageFormat::PNG)?;
    Ok(())
}
