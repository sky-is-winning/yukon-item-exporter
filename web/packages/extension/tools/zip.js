import fs from "fs/promises";
import path from "path";
import url from "url";
import archiver from "archiver";

/**
 * @param {string} source
 * @param {string} destination
 */
async function zip(source, destination) {
    await fs.mkdir(path.dirname(destination), { recursive: true });
    const output = (await fs.open(destination, "w")).createWriteStream();
    const archive = archiver("zip");

    output.on("close", () => {
        console.log(
            `Extension is ${archive.pointer()} total bytes when packaged.`,
        );
    });

    archive.on("error", (error) => {
        throw error;
    });

    archive.on("warning", (error) => {
        if (error.code === "ENOENT") {
            console.warn(`Warning whilst zipping extension: ${error}`);
        } else {
            throw error;
        }
    });

    archive.pipe(output);

    archive.directory(source, "");

    await archive.finalize();
}

const assets = url.fileURLToPath(new URL("../assets/", import.meta.url));
await zip(assets, /** @type {string} */ (process.argv[2]));
