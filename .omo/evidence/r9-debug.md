# R9 integration diagnosis

The first real automation run overlapped the general Electron suite and failed
at the submit event. An isolated run passed input and screenshot checks, then
timed out at the 1500-item snapshot. Neither failure is yet attributed to product
code. Full gate, 361 unit tests, general Electron suite and native MCP bridge pass.

Temporary artifact: command timing instrumentation in browserAutomation.cjs.
Remove it after locating the delayed operation. Preserve all existing assertions
and deadlines. No model turns or external page/account access.

The trace confirmed text insertion but no DOM key events despite acknowledged
CDP keyDown/keyUp and native/widget focus both true. Enabling CDP focus emulation
alone made the unchanged submit and 1500-item snapshot assertions pass.
The production fix initializes focus emulation and removes the type action's
pointer movement workaround. All temporary fixture instrumentation is removed.
Final production validation failed again at submit, so the focus-emulation fix
was reverted. The experiment did not establish it as sufficient.

Next temporary fixture artifact: wait for an actual compositor frame before the
blur/input scenario. Chromium source identified paint holding as a keyboard
dispatch rejection path independent of native focus. Existing assertions remain.

The compositor experiment received no frame and did not establish that cause.
Splitting the combined keyDown into rawKeyDown plus char, followed by keyUp,
passed the unchanged real fixture. The production implementation now uses that
explicit sequence and maps rawKeyDown to Electron's keyDown event for its narrow
synthetic-input guard. The fixture is restored byte-for-byte, without wrappers.
Type checks, all 33 automation unit tests and the complete real automation fixture
passed on the production fix. Human input still pauses control and protected
inputs remain rejected. No deadline or assertion was weakened.
