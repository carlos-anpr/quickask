fn main() -> anyhow::Result<()> {
    for arg in std::env::args().skip(1) {
        let image = image::open(&arg)?.to_rgb8();
        let count = image.pixels().filter(|p| p.0 == [255, 0, 255]).count();
        println!("{arg}: {count}");
    }
    Ok(())
}
