fn main() {
    // Generate `art::*` from the project, and bake the file a web build loads.
    pixelle_build::Art::new("assets/run2d")
        .bake_to("assets/run2d.pxlb")
        .run()
        .expect("the art builds");
}
