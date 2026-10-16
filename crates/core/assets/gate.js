// Whether a member's browser tool call may go ahead while its browser is in
// the person's hands — a pure rule, written into the browser host
// (`browser-host.cjs`) and tested on its own (`browser::tests`).
//
// `lane`: { control: 'person' | null, reason: 'check' | 'hand' | null,
//           touched: bool, page: the tab the hold is about (the page that
//           waits for a person, or the one the person is on), since: ms }
// `call`: { tool, args, targets: the tabs the call acts on }
//
// `null`: go ahead. Otherwise `{ text, wants, page }`: what the member is
// told, and — a close the person can allow with one click — `wants: 'close'`.
//
// - A hold equipe made for a page that waits for a person, nobody's hand on
//   it yet, never stops the member leaving that page — closing it, going
//   elsewhere in it — or using its other tabs: a check it no longer needs is
//   its to dismiss (the daemon then lets the hold go).
// - The person's own hand (they took it, or touched the page) makes that one
//   tab theirs: the member's other tabs, and closing them, stay its own.
function gate(lane, call) {
  if (!lane || lane.control !== 'person') return null;
  const targets = call.targets || [];
  const held = lane.page ?? null;
  const onIt = held === null ? targets.length > 0 : targets.includes(held);
  if (!onIt) return null;
  const closing = call.tool === 'browser_close' || (call.tool === 'browser_tabs' && call.args && call.args.action === 'close');
  if (lane.reason === 'check' && !lane.touched) {
    const leaves = closing || ['browser_navigate', 'browser_navigate_back', 'browser_tabs'].includes(call.tool);
    if (leaves) return null;
    return {
      text: 'This tab waits for a person (a "verify you are human" check, a sign-in or a code from their phone), and the person '
        + 'was asked to do it. Don\'t try to get around it here. You may close this tab or go elsewhere in it if you no longer need '
        + 'it, and your other tabs are yours; otherwise end your turn saying you wait for them — you\'ll be told when they hand it back.',
      wants: null,
      page: held,
    };
  }
  const at = lane.since ? new Date(lane.since) : null;
  const pad = (n) => String(n).padStart(2, '0');
  const since = at ? ` since ${pad(at.getHours())}:${pad(at.getMinutes())}` : '';
  return {
    text: `The person is using this tab${since} — ask them in words, or wait. Your other tabs are yours to use.`
      + (closing ? ' They were asked whether to let you close it.' : ''),
    wants: closing ? 'close' : null,
    page: held,
  };
}
