import React from "react";
import { AboutSettings } from "./AboutSettings";

/**
 * Body of the "About Talkkeeper" dialog (sidebar logo). It shows the same
 * panel as Settings → About, so both places say the same thing.
 */
export function About() {
    return (
        <div className="p-4 max-h-[80vh] overflow-y-auto">
            <AboutSettings />
        </div>
    );
}
