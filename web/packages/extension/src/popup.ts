import * as utils from "./utils";
import type { Options } from "./common";
import { bindOptions } from "./common";
import { buildInfo } from "ruffle-core";

let activeTab: chrome.tabs.Tab | browser.tabs.Tab;
let savedOptions: Options;
let tabOptions: Options;

let statusIndicator: HTMLDivElement;
let statusText: HTMLSpanElement;
let reloadButton: HTMLButtonElement;

// prettier-ignore
const STATUS_COLORS = {
    "status_init": "gray",
    "status_message_init": "gray",
    "status_no_tabs": "red",
    "status_result_disabled": "gray",
    "status_result_error": "red",
    "status_result_optout": "gray",
    "status_result_protected": "gray",
    "status_result_running": "green",
    "status_result_running_protected": "green",
    "status_tabs_error": "red",
};

async function queryTabStatus(
    listener: (status: keyof typeof STATUS_COLORS) => void,
) {
    listener("status_init");

    let tabs: chrome.tabs.Tab[] | browser.tabs.Tab[];
    try {
        tabs = await utils.tabs.query({
            currentWindow: true,
            active: true,
        });

        if (tabs.length < 1) {
            listener("status_no_tabs");
            return;
        }

        if (tabs.length > 1) {
            throw new Error(
                `Got ${tabs.length} tabs in response to active tab query.`,
            );
        }
    } catch (e) {
        listener("status_tabs_error");
        return;
    }

    activeTab = tabs[0]!;

    // FIXME: `activeTab.url` returns `undefined` on Chrome as it requires the `tabs`
    // permission, which we don't set in `manifest.json5` because of #11098.
    const url = activeTab.url ? new URL(activeTab.url) : null;
    if (
        url &&
        url.origin === window.location.origin &&
        url.pathname === "/player.html"
    ) {
        listener("status_result_running_protected");
        return;
    }

    listener("status_message_init");

    let response;
    try {
        response = await utils.tabs.sendMessage(activeTab.id!, {
            type: "ping",
        });
    } catch (e) {
        listener("status_result_protected");
        reloadButton.disabled = true;
        return;
    }

    if (!response) {
        listener("status_result_error");
        return;
    }

    tabOptions = response.tabOptions;

    if (response.loaded) {
        listener("status_result_running");
    } else if (tabOptions.ruffleEnable) {
        listener("status_result_optout");
    } else {
        listener("status_result_disabled");
    }

    optionsChanged();
}

/**
 * Should only be called on data type objects without any "cyclic members" to avoid infinite recursion.
 */
function deepEqual(x: unknown, y: unknown): boolean {
    if (
        typeof x === "object" &&
        typeof y === "object" &&
        x !== null &&
        y !== null
    ) {
        // Two non-null objects.

        for (const [key, value] of Object.entries(x)) {
            if (!deepEqual(value, y[key as keyof typeof y])) {
                return false;
            }
        }

        for (const [key, value] of Object.entries(y)) {
            if (!deepEqual(value, x[key as keyof typeof x])) {
                return false;
            }
        }

        return true;
    } else {
        // Not two non-null objects.

        return x === y;
    }
}

function optionsChanged() {
    if (!tabOptions) {
        return;
    }

    const isDifferent = !deepEqual(savedOptions, tabOptions);
    reloadButton.disabled = !isDifferent;
}

function displayTabStatus() {
    queryTabStatus((status) => {
        statusIndicator.style.setProperty("--color", STATUS_COLORS[status]);
        statusText.textContent = utils.i18n.getMessage(status);
    });
}

window.addEventListener("DOMContentLoaded", () => {
    bindOptions((options) => {
        savedOptions = options;
        optionsChanged();
    });

    statusIndicator = document.getElementById(
        "status-indicator",
    ) as HTMLDivElement;
    statusText = document.getElementById("status-text") as HTMLSpanElement;

    const versionText = document.getElementById(
        "version-text",
    ) as HTMLDivElement;
    versionText.textContent = `Ruffle extension ${buildInfo.versionName}`;

    const optionsButton = document.getElementById(
        "options-button",
    ) as HTMLButtonElement;
    optionsButton.textContent = utils.i18n.getMessage("open_settings_page");
    optionsButton.addEventListener("click", async () => {
        await utils.openOptionsPage();
        window.close();
    });

    const playerButton = document.getElementById(
        "player-button",
    ) as HTMLButtonElement;
    playerButton.textContent = utils.i18n.getMessage("open_player_page");
    playerButton.addEventListener("click", async () => {
        await utils.openPlayerPage();
        window.close();
    });

    reloadButton = document.getElementById(
        "reload-button",
    ) as HTMLButtonElement;
    reloadButton.textContent = utils.i18n.getMessage("action_reload");
    reloadButton.addEventListener("click", async () => {
        await utils.tabs.reload(activeTab.id!);
        window.close();
    });

    displayTabStatus();
});
