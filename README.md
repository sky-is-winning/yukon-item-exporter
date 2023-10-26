# Yukon Item Exporter

This repo uses Ruffle to export frames for items, so they can be used in the Yukon client or any similar project. I will not provide ANY support for this, and it is a fairly complicated process. Please read the instructions VERY CAREFULLY. 

## Prerequisites

- Rust
- Java
- Node.js
- Set your environment variables correctly using:
`set RUST_LOG=avm_trace=trace`

## How to use

- Run the command 
`cargo run --release --package=ruffle_desktop -- export-script.swf`
- Enter the swf you want to export into the text field, and click on the button.
- When it has gone through every frame, close the Ruffle desktop app but do NOT close the Command Prompt/Terminal.
- Copy all the trace output from the Command Prompt/Terminal into ruffle_trace_output.txt. That's every line that includes 'INFO avm_trace'
- Run the command
`node rename-screenshots.js`
- All your frames will now be in the exported_frames folder, and you can pack them using Texture Packer