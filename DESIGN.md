# Editor visual system

Operate mode. Preserve the accepted dark DAW arrangement rather than inventing a new workflow.

Slate background, slightly lighter panels, quiet one-pixel dividers, crisp high-contrast text. Cyan marks active controls and the playhead. Lane hits use the required blue, purple and red volume colours. Errors use muted red with explicit recovery text.

Top: menus and compact labelled ribbon. Left: sample browser. Centre: timeline ruler, waveform, lanes and volume controller. Right: inspector and history. Bottom: a 56-pixel transport. Panels may be hidden and must release their space. Minimum readable editing layout at 1024x720; starting size 1440x900.

Built with Svelte 5 around a `<canvas>` timeline (Tauri v2). Use standard control focus and keyboard conventions. Text fields retain editing keys. Empty state explains how to open a map, with a prominent Open difficulty action. Errors name the failed action and preserve the open document. No decorative cards or fabricated maps in the default editor.
