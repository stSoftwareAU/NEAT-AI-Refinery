/**
 * @module
 *
 * The **golden reference sampler** — GRQ's `src/train/Sampler.ts` algorithm,
 * extracted so it can run against a fixture corpus.
 *
 * GRQ's own entry point cannot be executed here: it resolves the record shape
 * through `NetworkUtil`/`VersionManager` and the corpus directory through GRQ
 * version state, so it needs a creature export and a GRQ checkout to start at
 * all. What it does to the bytes, however, is self-contained, and that is what
 * this file reproduces line for line:
 *
 * 1. list the `.bin` files in the source directory (`Deno.readDirSync`);
 * 2. Fisher-Yates shuffle that file list;
 * 3. stream each file a record at a time, keeping a record when
 *    `Math.random() < rate`;
 * 4. Fisher-Yates shuffle the records kept **from that file** and append them;
 * 5. write `sample-<Math.round(rate * 100)>.bin` into a staging directory;
 * 6. publish it with rename-aside → rename-in → remove-aside.
 *
 * Replaced seams, all of them environmental rather than behavioural:
 *
 * | GRQ | Here |
 * | --- | --- |
 * | `NetworkUtil` / `VersionManager` supply the shape and corpus path | `--inputs`, `--outputs`, `--source`, `--output` |
 * | `getLogger()` diagnostics | a JSON summary on stdout |
 * | `writeRecordsOrThrow` ENOSPC diagnostics | the same short-write loop, without the free-space report |
 * | `reclaimSamplerScratch` / exit code 28 | the staging directory this run created is removed |
 * | staging under `.tmp/` relative to the working directory | staging beside the output, so a fixture run stays in its own directory |
 *
 * Provenance: stSoftwareAU/GRQ commit `3ae5a5987bd0c1bc115dd83c50854a574d806b51`,
 * `src/train/Sampler.ts` sha256
 * `e0b2670c70b6e3526a552d69a8908410ace80afd0571661a81f21756ea6c18db`.
 *
 * Usage:
 *
 * ```bash
 * deno run --allow-read --allow-write grq_sampler.ts \
 *   --source <dir> --output <dir> --inputs 2 --outputs 1 --rate 0.05
 * ```
 */

interface Options {
  source: string;
  output: string;
  inputs: number;
  outputs: number;
  rate: number;
}

/** The counts the harness reads back off stdout. */
interface Summary {
  outputFile: string;
  recordsRead: number;
  recordsWritten: number;
  sources: string[];
}

/** Parses `--key value` pairs, failing loud on anything unrecognised. */
function parseOptions(argv: string[]): Options {
  const values = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key.startsWith("--") || value === undefined) {
      throw new Error(`expected --key value pairs, got: ${argv.join(" ")}`);
    }
    values.set(key.slice(2), value);
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
    source: required("source"),
    output: required("output"),
    inputs: count("inputs"),
    outputs: count("outputs"),
    rate: Number(required("rate")),
  };
}

/** Fisher-Yates over the records kept from one file — GRQ verbatim. */
function shuffleUint8Array(array: Uint8Array[]) {
  for (let i = array.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [array[i], array[j]] = [array[j], array[i]];
  }
}

/** Fisher-Yates over the input file list — GRQ verbatim. */
function shuffleStrings(array: string[]) {
  for (let i = array.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [array[i], array[j]] = [array[j], array[i]];
  }
}

/**
 * GRQ's `writeRecordsOrThrow` short-write loop, minus the ENOSPC diagnostic
 * that reports free space: a write that makes no progress still fails loud.
 */
function writeRecords(
  outFile: Deno.FsFile,
  records: Uint8Array[],
  outPath: string,
) {
  for (let index = 0; index < records.length; index++) {
    let remaining = records[index];
    while (remaining.length > 0) {
      const written = outFile.writeSync(remaining);
      if (written <= 0) {
        throw new Error(
          `no progress writing record ${
            index + 1
          }/${records.length} to ${outPath}`,
        );
      }
      remaining = remaining.subarray(written);
    }
  }
}

/**
 * GRQ's `readAndProcess`: keep each record with probability `rate`, shuffle
 * the survivors of this file, append them. Returns the records read and kept.
 */
function readAndProcess(
  filePath: string,
  outFile: Deno.FsFile,
  outPath: string,
  options: Options,
): { read: number; kept: number } {
  const valuesCount = options.inputs + options.outputs;
  const BYTES_PER_RECORD = valuesCount * 4; // Each float is 4 bytes
  const readFile = Deno.openSync(filePath, { read: true });

  function readNextRecord() {
    const array = new Float32Array(valuesCount);
    const uint8Array = new Uint8Array(array.buffer);
    const bytesRead = readFile.readSync(uint8Array);
    if (bytesRead === null || bytesRead === 0) {
      return null;
    }
    if (bytesRead !== BYTES_PER_RECORD) {
      throw new Error(
        `Invalid number of bytes read ${bytesRead} expected ${BYTES_PER_RECORD}`,
      );
    }

    return { values: uint8Array };
  }

  const sampledRecords: Uint8Array[] = [];
  let read = 0;
  while (true) {
    const result = readNextRecord();

    if (result === null) break;

    read++;
    if (Math.random() < options.rate) {
      sampledRecords.push(result.values);
    }
  }

  readFile.close();

  // Shuffle all sampled records for final output randomization
  shuffleUint8Array(sampledRecords);
  writeRecords(outFile, sampledRecords, outPath);

  return { read, kept: sampledRecords.length };
}

/** GRQ's `publishSamplerDir` — rename aside, rename in, remove aside. */
function publishSamplerDir(tmpDir: string, finalDir: string): void {
  const asidePath = `${finalDir}.deleting-${Date.now()}-${Deno.pid}`;
  let renamedAside = false;

  try {
    try {
      Deno.renameSync(finalDir, asidePath);
      renamedAside = true;
    } catch (err) {
      if (!(err instanceof Deno.errors.NotFound)) {
        throw err;
      }
    }

    Deno.renameSync(tmpDir, finalDir);
  } catch (err) {
    if (renamedAside) {
      try {
        Deno.renameSync(asidePath, finalDir);
      } catch (_rollbackErr) {
        // Best effort; surface the original error.
      }
    }
    throw err;
  }

  if (renamedAside) {
    Deno.removeSync(asidePath, { recursive: true });
  }
}

/** The directory holding `path`, or `.` when it has no parent component. */
function parentOf(path: string): string {
  const cut = path.replace(/\/+$/, "").lastIndexOf("/");
  return cut > 0 ? path.slice(0, cut) : ".";
}

function loader(options: Options): Summary {
  const finalDir = options.output;
  const tmpDir = `${parentOf(finalDir)}/.tmp/sampler-${Date.now()}-${Deno.pid}`;
  try {
    Deno.mkdirSync(tmpDir, { recursive: true });

    const files: string[] = [];
    for (const dirEntry of Deno.readDirSync(options.source)) {
      if (dirEntry.isFile && dirEntry.name.endsWith(".bin")) {
        files.push(dirEntry.name);
      }
    }

    // Shuffle the list of files for randomized processing
    shuffleStrings(files);

    const percent = Math.round(options.rate * 100);
    const outPath = `${tmpDir}/sample-${percent}.bin`;
    const outFile = Deno.openSync(outPath, {
      write: true,
      create: true,
      truncate: true,
    });

    let recordsRead = 0;
    let recordsWritten = 0;
    for (const file of files) {
      const counts = readAndProcess(
        `${options.source}/${file}`,
        outFile,
        outPath,
        options,
      );
      recordsRead += counts.read;
      recordsWritten += counts.kept;
    }
    outFile.close();

    publishSamplerDir(tmpDir, finalDir);

    return {
      outputFile: `${finalDir}/sample-${percent}.bin`,
      recordsRead,
      recordsWritten,
      sources: files,
    };
  } catch (e) {
    // Reclaim only the scratch this run created; the previously published
    // corpus is left exactly as it was.
    try {
      Deno.removeSync(tmpDir, { recursive: true });
    } catch (_cleanupErr) {
      // Already gone — the publish moved it.
    }
    throw e;
  }
}

if (import.meta.main) {
  const options = parseOptions(Deno.args);
  if ((options.rate > 0 && options.rate <= 1) == false) {
    console.error(`Invalid sample rate ${options.rate}`);
    Deno.exit(1);
  }

  try {
    console.log(JSON.stringify(loader(options)));
  } catch (e) {
    console.error("Sampler failed", e);
    Deno.exit(1);
  }
}
