import { renderFixture } from "./render.mjs";

export const FIXTURE_MESSAGE = "hello from playwright";

export default async function globalSetup() {
  // Build any fixtures the spec files reference. Idempotent.
  for (const profile of ["audible", "ultrasonic"]) {
    const p = renderFixture(profile, FIXTURE_MESSAGE);
    console.log(`fixture[${profile}] = ${p}`);
  }
}
