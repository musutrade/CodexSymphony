fn main() {
    // SQLx embeds migrations at compile time; a new file must rebuild the binary.
    println!("cargo:rerun-if-changed=../../migrations");
}
