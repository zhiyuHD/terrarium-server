<!-- Owner: ../web/index.html, ../web/scene.js, ../scripts/build-scene.mjs -->
# Gallery animation

Only canvases intersecting the viewport initialize their scenes. Offscreen and hidden-page scenes pause while retaining their latest image and state; removing a card disposes its scene. Scroll and touch-move listeners are passive, pause gallery and replay animation, and resume after 180 ms without a new event. Pending snapshots apply once on resume. Mobile or coarse-pointer previews draw at most 12 frames per second; other previews use 24. Land and water share the renderer's frame scheduler.

Rebuild the browser bundle with independent World and renderer checkouts:

```sh
node scripts/build-scene.mjs --world /path/to/cortico-world-terrarium --renderer /path/to/terrarium-renderer
```

Install the photo build dependencies first; the script uses its esbuild dependency. The bundle contains the World display adapter and the renderer without runtime imports from either checkout. Install and activate a new Native release to publish the updated HTML and script together. Retained release assets keep open pages on a compatible bundle.
