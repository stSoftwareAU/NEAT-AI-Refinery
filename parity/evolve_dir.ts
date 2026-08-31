/**
 * @module
 *
 * End-to-end consumer check: can NEAT-AI's `Creature.evolveDir` open and use a
 * corpus directory that Refinery published, with no adaptation on either side?
 *
 * The corpus is handed over exactly as `neat_ai_refinery … sample` published
 * it — same directory, same `sample-<percent>.bin` name, same fixed-width
 * records — and a small creature is evolved over it for a single generation.
 * A run that reads the corpus reports a finite error; a directory NEAT-AI
 * cannot consume raises instead.
 *
 * ```bash
 * deno run --allow-read --allow-write --allow-env --allow-run --allow-sys \
 *   --allow-net=jsr.io \
 *   evolve_dir.ts --corpus <dir> --inputs 2 --outputs 1 [--expect-failure]
 * ```
 *
 * NEAT-AI fetches its WASM activation bundle from `jsr.io` the first time it
 * runs on a machine and serves it from its own cache afterwards, so the net
 * permission is needed on a clean checkout even though a warm machine never
 * uses it.
 *
 * On success a single JSON object is printed on stdout:
 * `{"consumed":true,"error":0.0474…,"generations":1}`. `--expect-failure`
 * inverts the check — it is the harness's control, proving the positive
 * assertion is sensitive to a corpus NEAT-AI cannot read.
 */

import { Creature } from "@stsoftware/neat-ai";

interface Options {
  corpus: string;
  inputs: number;
  outputs: number;
  expectFailure: boolean;
}

function parseOptions(argv: string[]): Options {
  const values = new Map<string, string>();
  let expectFailure = false;
  for (let index = 0; index < argv.length; index++) {
    const key = argv[index];
    if (key === "--expect-failure") {
      expectFailure = true;
      continue;
    }
    const value = argv[index + 1];
    if (!key.startsWith("--") || value === undefined) {
      throw new Error(`expected --key value pairs, got: ${argv.join(" ")}`);
    }
    values.set(key.slice(2), value);
    index++;
  }

  const required = (key: string): string => {
    const value = values.get(key);
    if (value === undefined) throw new Error(`missing --${key}`);
    return value;
  };
  const count = (key: string): number => {
    const value = Number(required(key));
    if (!Number.isInteger(value) || value < 1) {
      throw new Error(`--${key} must be a positive integer, got ${value}`);
    }
    return value;
  };

  return {
    corpus: required("corpus"),
    inputs: count("inputs"),
    outputs: count("outputs"),
    expectFailure,
  };
}

/**
 * Evolves a small creature over `corpus` for one generation and returns the
 * error NEAT-AI computed from the records it read.
 */
async function consume(options: Options): Promise<{
  error: number;
  generations: number;
}> {
  const creature = new Creature(options.inputs, options.outputs, {
    layers: [{ count: 2 }],
  });
  const result = await creature.evolveDir(options.corpus, {
    iterations: 1,
    populationSize: 4,
    threads: 1,
    targetError: 0.5,
    log: 1,
  });

  if (!Number.isFinite(result.error)) {
    throw new Error(
      `evolveDir returned a non-finite error: ${result.error} — the corpus was not scored`,
    );
  }

  return { error: result.error, generations: result.generation ?? 0 };
}

/**
 * Is `e` — or anything it was caused by — a sandbox fault rather than a
 * verdict on the corpus?
 *
 * The control below asserts that NEAT-AI *rejects* a malformed corpus, and a
 * missing permission raises from the same `await`. Without this check a
 * runner that denied net access would report the rejection the control is
 * looking for and the gate would pass on an environment fault.
 */
function isEnvironmentFault(e: unknown): boolean {
  for (let cause = e; cause instanceof Error; cause = cause.cause) {
    if (
      cause instanceof Deno.errors.NotCapable ||
      cause instanceof Deno.errors.PermissionDenied
    ) {
      return true;
    }
  }
  return false;
}

if (import.meta.main) {
  const options = parseOptions(Deno.args);

  if (options.expectFailure) {
    try {
      await consume(options);
    } catch (e) {
      if (isEnvironmentFault(e)) {
        console.error(
          `evolveDir could not run: ${
            e instanceof Error ? e.message : String(e)
          } — this is a sandbox fault, not a rejected corpus`,
        );
        Deno.exit(1);
      }
      console.log(
        JSON.stringify({
          consumed: false,
          rejected: e instanceof Error ? e.message : String(e),
        }),
      );
      Deno.exit(0);
    }
    console.error(
      `evolveDir consumed ${options.corpus}, which it should have rejected`,
    );
    Deno.exit(1);
  }

  const outcome = await consume(options);
  console.log(JSON.stringify({ consumed: true, ...outcome }));
}
