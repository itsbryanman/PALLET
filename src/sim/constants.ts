// Gameplay tuning. Locked constants: identical recorded inputs must produce
// identical incidents (tests/sim.test.ts replays them).

export const DT = 1 / 60;

export const MAX_SPEED = 190; // px/s forward
export const MAX_REVERSE = 95;
export const ACCEL = 300;
export const REVERSE_ACCEL = 200;
export const BRAKE = 620;
export const COAST = 170;
export const PRECISION = 0.35; // Shift caps speed to this fraction

export const WHEELBASE = 30;
export const MAX_STEER = 0.62; // rad
export const STEER_RATE = 5.5; // steering input approach rate (1/s)
export const STEER_RETURN = 7;

export const FORK_RATE = 1.8; // fork height units per second

// Tip rule (GAME_DESIGN.md): above 70% top speed, forks raised, steering past
// 60%, sustained for 250 ms. Taller and heavier stacks lower both thresholds.
export const TIP_SPEED = 0.7;
export const TIP_STEER = 0.6;
export const TIP_HOLD = 0.25; // s
export const TIP_FORK = 0.5;
export const STACK_PENALTY_SPEED = 0.04; // per crate above one
export const STACK_PENALTY_STEER = 0.03;
export const MASS_PENALTY = 0.012; // per mass unit above one, summed over the stack

// Hitting a wall above this fraction of top speed with a load tips it.
export const WALL_TIP_SPEED = 0.8;
export const BUMP_SPEED = 0.25;

// Forklift body (rotated rectangle approximated by a circle for walls).
export const BODY_RADIUS = 19;
export const FORK_REACH = 38; // pivot to fork pickup point; matches the sprite frames at SPRITE_SCALE
export const PICK_RADIUS = 20;
export const PICK_ALIGN = 0.45; // rad
