mod localize;

#[cfg(feature = "lopdf")]
mod lopdf;

#[cfg(feature = "mupdf")]
mod mupdf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "lopdf")]
    return lopdf::main();

    #[cfg(feature = "mupdf")]
    return mupdf::main();
}
