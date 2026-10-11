import type { PageTurn } from "./pageGesture";

export type PageTurnSource = "discrete" | "keyboard";

export type QueuedPageTurn = {
  direction: PageTurn;
  repeated: boolean;
  source: PageTurnSource;
};

export function enqueuePageTurn(
  queue: QueuedPageTurn[],
  turn: QueuedPageTurn,
  maximum: number,
) {
  const last = queue.at(-1);
  if (last && last.direction !== turn.direction) {
    queue.pop();
  } else if (turn.repeated && queue.some((item) => item.repeated && item.source === turn.source)) {
    return;
  } else if (queue.length < maximum) {
    queue.push(turn);
  }
}

export function releasePageTurnSource(queue: QueuedPageTurn[], source: PageTurnSource) {
  return queue.filter((turn) => turn.source !== source);
}
