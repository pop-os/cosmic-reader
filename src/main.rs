#[cfg(feature = "lopdf")]
mod lopdf;

#[cfg(feature = "poppler")]
mod poppler;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "lopdf")]
    return lopdf::main();

    #[cfg(feature = "poppler")]
    return poppler::main();
}
