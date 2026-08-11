fn main() {
    uniffi::generate_scaffolding("src/ext4_bridge.udl").unwrap_or_else(|_| {
        // Fallback for macro-only scaffolding if UDL is not used
    });
}
