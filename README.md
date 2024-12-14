# Yukon Item Exporter

This repo uses Ruffle to export frames for items, so they can be used in the Yukon client or any similar project.

I recently massively overhauled this process, making it way easier. Just navigate to the releases section to download the installer, drag in your SWFs and you're good to go.

## Building from source:

### Prerequisites

- Rust
- Java

### How to use (if not using the GUI packaged in the installer)

- Run the command
  `cargo run --release --package=ruffle_desktop -- --dummy-external-interface --no-gui --export-swf=YOUR_SWF_NAME.swf --stage-width=150 --stage-scale=2 export-script.swf`
- Wait for the program to complete
- All your frames will now be in the exported_frames folder, and you can pack them using Texture Packer
