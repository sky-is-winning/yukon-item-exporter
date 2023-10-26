const fs = require('fs');

const screenshots = fs.readdirSync('./ruffle_screenshots');
const ruffle_trace_output = fs.readFileSync('./ruffle_trace_output.txt', 'utf8').split('\r\n');

for (let i = 0; i < ruffle_trace_output.length; i++) {
    const line = ruffle_trace_output[i];
    let output = line.split('avm_trace: ')[1];
    let timestamp = parseInt(output.split(': frame')[0]);
    let frame = output.split(': frame ')[1];
    for (let j = 0; j < screenshots.length; j++) {
        const screenshot = screenshots[j];
        let screenshot_timestamp = parseInt(screenshot.split('.')[0]);
        if (screenshot_timestamp >= timestamp) {
            fs.renameSync(`./ruffle_screenshots/${screenshot}`, `./exported_frames/${frame}.png`);
            break;
        }
    }
}

for (let screenshot of fs.readdirSync('./ruffle_screenshots')) {
    fs.unlinkSync(`./ruffle_screenshots/${screenshot}`);
}