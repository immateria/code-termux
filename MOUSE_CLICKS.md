# Mouse Click Support

## Summary

Added mouse click support for UI elements in the TUI. Users can now click on header bar elements to interact with settings.

## Implemented Features

### Header Bar Clicks

1. **Model Selector**
   - Click on "Model: [current-model]" in the header
   - Opens the interactive model selector (same as typing `/model`)

2. **Shell Selector**
   - Click on "Shell: [current-shell]" in the header
   - Opens the interactive shell selector (same as typing `/shell`)

3. **Reasoning Effort Cycling**
   - Click on "Reasoning: [current-level]" in the header
   - Cycles through reasoning efforts: None → Minimal → Low → Medium → High → XHigh → None

## Implementation Details

### Architecture

- **ClickableRegion**: Struct that pairs a screen `Rect` with a `ClickableAction`
- **Tracking**: Clickable regions are calculated during render and stored in `ChatWidget.clickable_regions`
- **Hit Testing**: On left mouse click, coordinates are checked against stored regions
- **Action Dispatch**: Matching regions trigger their associated actions

### Files Modified

- `code-rs/tui/src/chatwidget.rs`:
  - Added `ClickableAction` enum and `ClickableRegion` struct
  - Added `clickable_regions: RefCell<Vec<ClickableRegion>>` field to `ChatWidget`
  - Added `track_status_bar_clickable_regions()` method to calculate header regions
  - Extended `handle_mouse_event()` to handle `MouseEventKind::Down`
  - Added `handle_click()` method to dispatch actions for clicked regions

- `code-rs/tui/src/tui.rs`:
  - Added `EnableMouseCapture` to terminal initialization (already done)

### Centering Calculation

The header bar uses centered text alignment, so clickable regions must account for:
1. Total width of all spans
2. Starting x position: `area.x + ((area.width - total_width) / 2)`
3. Tracking each span's position relative to the centered start

## Future Work

### Command Popup Clicks (Not Implemented)

Clicking on slash command popup items would require:
1. Exposing command popup state from `BottomPane`/`ChatComposer`
2. Calculating popup item positions based on scroll state
3. Tracking dynamic regions that change as user types

This is deferred due to complexity - the popup is rendered deep in the `BottomPane` hierarchy with private state.

### Potential Enhancements

- Add visual hover effects (requires terminal hover event support)
- Add click handlers for other UI elements (e.g., Branch, Directory in header)
- Support right-click context menus
- Add clickable links in history cells (e.g., file paths, URLs)

## Testing

To test the implementation:

1. Run the TUI: `./code-rs/target/debug/code`
2. Click on "Model:" in the top header bar - selector should open
3. Click on "Shell:" in the top header bar - shell selector should open
4. Click on "Reasoning:" in the top header bar - effort level should cycle
5. Verify Shift+click still allows terminal text selection (not captured)

## Notes

- Shift-modified mouse events are deliberately ignored to allow terminal text selection
- Only left mouse button clicks are handled
- Click regions are recalculated on every render to account for dynamic content
- Click handling uses borrow checker-safe patterns (clone action before executing)
