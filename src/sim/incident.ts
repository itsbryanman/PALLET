// The fictional incident report. Deterministic: the same incident number and
// circumstances always produce the same paperwork.

export interface Incident {
  number: number;
  cause: "turn" | "wall";
  speedKmh: number;
  cargo: string[];
  forksRaised: boolean;
}

const TURN = [
  "Took the corner like it owed money.",
  "Steering input described by witnesses as \"enthusiastic.\"",
  "Attempted a three-point turn in one point.",
  "Confused a forklift with a rally car.",
];
const WALL = [
  "Wall did not yield.",
  "Operator tested the wall. Wall passed.",
  "Unscheduled structural inspection.",
  "Braking was considered, then deferred.",
];
const ACTIONS = [
  "Operator to watch the safety video again, with feeling.",
  "Forks low when driving. Write it on your hand.",
  "Slow down near the bays. The bays aren't going anywhere.",
  "Re-read page one of the manual. There is only page one.",
];

export function incidentLines(i: Incident): [string, string][] {
  const pick = (list: string[], salt: number) => list[(i.number * 7 + salt) % list.length];
  const cargo = i.cargo.length === 0 ? "None (forks empty)" : i.cargo.length === 1 ? i.cargo[0] : `${i.cargo[0]} and ${i.cargo.length - 1} more`;
  return [
    ["Cargo", cargo],
    ["Speed at incident", `${i.speedKmh.toFixed(1)} km/h`],
    ["Forks", i.forksRaised ? "Raised. That's the problem." : "Lowered"],
    ["Probable cause", pick(i.cause === "turn" ? TURN : WALL, 1)],
    ["Injuries", "Operator's pride"],
    ["Files damaged", "0"],
    ["Corrective action", pick(ACTIONS, 3)],
  ];
}

export function incidentText(i: Incident, at: Date, locations: string[]): string {
  const lines = incidentLines(i).map(([k, v]) => `${k.padEnd(18)} ${v}`);
  return [
    "FICTIONAL WAREHOUSE INCIDENT REPORT",
    `Form PAL-1, incident no. ${String(i.number).padStart(4, "0")}`,
    `Filed ${at.toISOString()}`,
    "",
    ...lines,
    "",
    "Where the files actually are (unchanged by this incident):",
    ...(locations.length ? locations.map((l) => `  ${l}`) : ["  (no cargo)"]),
    "",
    "This is a joke from PALLET, a file manager with a forklift.",
    "It is not an official form and no agency was notified.",
    "",
  ].join("\n");
}
