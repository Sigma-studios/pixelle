fn main() {
    // Bake the runtime file release builds load, and generate `art::*` from the project.
    pixelle_build::Art::new("assets/run2d.pxl")
        .bake_to("assets/run2d.pxlb")
        .run()
        .expect("the art builds");
}
