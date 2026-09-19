#!/usr/bin/env node

import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const reportsDirectory = path.join(root, 'benchmarks/reports');
const docs = {
  en: path.join(root, 'docs/PERFORMANCE-BASELINE.md'),
  zh: path.join(root, 'docs/PERFORMANCE-BASELINE.zh-CN.md'),
};
const siteDocs = {
  en: path.join(root, 'site/public/source/docs/PERFORMANCE-BASELINE.md'),
  zh: path.join(root, 'site/public/source/docs/PERFORMANCE-BASELINE.zh-CN.md'),
};
const startMarker = '<!-- BEGIN GENERATED PERFORMANCE REPORTS -->';
const endMarker = '<!-- END GENERATED PERFORMANCE REPORTS -->';
const generatedPathspecs = [
  ':(exclude)benchmarks/reports/**',
  ':(exclude)docs/PERFORMANCE-BASELINE.md',
  ':(exclude)docs/PERFORMANCE-BASELINE.zh-CN.md',
  ':(exclude)site/public/source/docs/PERFORMANCE-BASELINE.md',
  ':(exclude)site/public/source/docs/PERFORMANCE-BASELINE.zh-CN.md',
];

function usage() {
  console.log(`Usage: node scripts/record-performance.mjs [options]

Runs the local performance suite, writes one JSONL report, and regenerates the
English and Chinese performance documents from benchmarks/reports/*.jsonl.

Options:
  --release VERSION     Write <source>-VERSION.jsonl instead of <source>-last.jsonl
  --reuse-results NAME  Reuse a Criterion baseline; artifact sizes are remeasured
  --generate-only       Regenerate documents without recording a new report
  -h, --help            Show this help`);
}

const options = {generateOnly: false, releaseVersion: undefined, reuseBaseline: undefined};
for (let index = 2; index < process.argv.length; index += 1) {
  const argument = process.argv[index];
  if (argument === '--help' || argument === '-h') {
    usage();
    process.exit(0);
  }
  if (argument === '--generate-only') {
    options.generateOnly = true;
    continue;
  }
  if (argument === '--release' && process.argv[index + 1]) {
    options.releaseVersion = process.argv[++index];
    continue;
  }
  if (argument === '--reuse-results' && process.argv[index + 1]) {
    options.reuseBaseline = process.argv[++index];
    continue;
  }
  console.error(`unknown or incomplete argument: ${argument}`);
  usage();
  process.exit(2);
}
if (options.generateOnly && (options.releaseVersion || options.reuseBaseline)) {
  throw new Error('--generate-only cannot be combined with recording options');
}
if (options.releaseVersion && !/^[0-9A-Za-z._-]+$/.test(options.releaseVersion)) {
  throw new Error(`invalid release version: ${options.releaseVersion}`);
}

function run(command, args, {capture = false} = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: 'utf8',
    stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
  });
  if (capture) {
    process.stdout.write(result.stdout ?? '');
    process.stderr.write(result.stderr ?? '');
  }
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited with status ${result.status}`);
  return result;
}

function commandOutput(command, args) {
  const result = spawnSync(command, args, {cwd: root, encoding: 'utf8'});
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status}: ${result.stderr}`);
  }
  return result.stdout.trim();
}

function removeNamedDirectories(directory, name) {
  if (!existsSync(directory)) return;
  for (const entry of readdirSync(directory, {withFileTypes: true})) {
    if (!entry.isDirectory()) continue;
    const child = path.join(directory, entry.name);
    if (entry.name === name) rmSync(child, {recursive: true, force: true});
    else removeNamedDirectories(child, name);
  }
}

function collectBenchmarks(directory, baselineName, results = new Map()) {
  if (!existsSync(directory)) return results;
  for (const entry of readdirSync(directory, {withFileTypes: true})) {
    const child = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      collectBenchmarks(child, baselineName, results);
      continue;
    }
    if (entry.name !== 'benchmark.json' || path.basename(path.dirname(child)) !== baselineName) {
      continue;
    }
    const metadata = JSON.parse(readFileSync(child, 'utf8'));
    const estimates = JSON.parse(
      readFileSync(path.join(path.dirname(child), 'estimates.json'), 'utf8'),
    );
    const estimate = estimates.slope?.point_estimate ?? estimates.mean.point_estimate;
    if (results.has(metadata.full_id)) {
      throw new Error(`duplicate benchmark result: ${metadata.full_id}`);
    }
    results.set(metadata.full_id, estimate);
  }
  return results;
}

function shanghaiDate(date = new Date()) {
  const parts = new Intl.DateTimeFormat('en-CA', {
    timeZone: 'Asia/Shanghai', year: 'numeric', month: '2-digit', day: '2-digit',
  }).formatToParts(date);
  const values = Object.fromEntries(parts.map(({type, value}) => [type, value]));
  return `${values.year}-${values.month}-${values.day}`;
}

function sourceIdentity() {
  const head = commandOutput('git', ['rev-parse', '--short=7', 'HEAD']);
  const pathspec = ['--', '.', ...generatedPathspecs];
  const staged = commandOutput('git', ['diff', '--cached', '--binary', 'HEAD', ...pathspec]);
  if (staged) {
    return {
      id: createHash('sha256').update(staged).digest('hex').slice(0, 12),
      kind: 'staged',
      baseRevision: head,
    };
  }
  const worktree = commandOutput('git', ['diff', '--binary', 'HEAD', ...pathspec]);
  if (worktree) {
    return {
      id: createHash('sha256').update(worktree).digest('hex').slice(0, 12),
      kind: 'worktree',
      baseRevision: head,
    };
  }
  return {id: head, kind: 'commit', baseRevision: head};
}

function writeJsonl(file, runRecord, benchmarks, artifacts) {
  const records = [runRecord];
  for (const [name, estimateNs] of [...benchmarks].sort(([a], [b]) => a.localeCompare(b))) {
    records.push({type: 'benchmark', name, estimate_ns: estimateNs});
  }
  for (const artifact of artifacts) records.push({type: 'artifact', ...artifact});
  mkdirSync(path.dirname(file), {recursive: true});
  const temporary = `${file}.tmp`;
  writeFileSync(temporary, `${records.map((record) => JSON.stringify(record)).join('\n')}\n`);
  renameSync(temporary, file);
}

function recordPerformance() {
  const baselineName = options.reuseBaseline ?? 'latest-unreleased-report';
  if (!/^[A-Za-z0-9._-]+$/.test(baselineName)) {
    throw new Error(`invalid Criterion baseline name: ${baselineName}`);
  }
  const sampleSize = process.env.VELIN_PERF_SAMPLE_SIZE ?? '20';
  const warmupTime = process.env.VELIN_PERF_WARMUP_TIME ?? '0.5';
  const measurementTime = process.env.VELIN_PERF_MEASUREMENT_TIME ?? '1';
  const criterionArgs = [
    '--noplot', '--sample-size', sampleSize, '--warm-up-time', warmupTime,
    '--measurement-time', measurementTime, '--save-baseline', baselineName,
    '--format', 'terse',
  ];

  if (!options.reuseBaseline) {
    removeNamedDirectories(path.join(root, 'target/criterion'), baselineName);
    run('cargo', ['bench', '-p', 'velin', '--bench', 'pipeline', '--', ...criterionArgs]);
    run('cargo', ['bench', '-p', 'velin-capi', '--bench', 'c_api', '--', ...criterionArgs]);
    run('cargo', [
      'bench', '-p', 'velin-wasm', '--features', 'runtime', '--bench', 'runtime',
      '--', ...criterionArgs,
    ]);
  }

  const benchmarks = collectBenchmarks(path.join(root, 'target/criterion'), baselineName);
  if (benchmarks.size === 0) {
    throw new Error(`no Criterion results found for baseline ${baselineName}`);
  }
  const sizeRun = run('bash', ['scripts/measure-embed-size.sh'], {capture: true});
  const sizes = new Map();
  const sizePattern = /^(.*?)\s+([0-9]+) bytes\s+([0-9]+) gzip\s+/gm;
  for (const match of (sizeRun.stdout ?? '').matchAll(sizePattern)) {
    sizes.set(match[1].trim(), {bytes: Number(match[2]), gzipBytes: Number(match[3])});
  }
  const gate = JSON.parse(
    readFileSync(path.join(root, 'benchmarks/performance-baseline.json'), 'utf8'),
  );
  const artifacts = Object.entries(gate.artifacts).map(([name, limits]) => {
    const measured = sizes.get(name);
    if (!measured) throw new Error(`missing artifact measurement: ${name}`);
    if (measured.bytes > limits.bytes || measured.gzipBytes > limits.gzip_bytes) {
      throw new Error(
        `${name} exceeds its size ceiling: ${measured.bytes}/${measured.gzipBytes} ` +
        `(limits ${limits.bytes}/${limits.gzip_bytes})`,
      );
    }
    return {
      name,
      bytes: measured.bytes,
      gzip_bytes: measured.gzipBytes,
      max_bytes: limits.bytes,
      max_gzip_bytes: limits.gzip_bytes,
    };
  });

  const cargoToml = readFileSync(path.join(root, 'Cargo.toml'), 'utf8');
  const workspaceVersion = cargoToml.match(
    /\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m,
  )?.[1];
  if (!workspaceVersion) throw new Error('could not read the workspace version');
  const rustVerbose = commandOutput('rustc', ['-Vv']);
  const identity = sourceIdentity();
  const kind = options.releaseVersion ? 'release' : 'unreleased';
  const version = options.releaseVersion ?? workspaceVersion;
  const date = shanghaiDate();
  const runRecord = {
    type: 'run',
    schema_version: 1,
    kind,
    version,
    source_id: identity.id,
    source_kind: identity.kind,
    base_revision: identity.baseRevision,
    recorded_at: new Date().toISOString(),
    date,
    ...(kind === 'release' ? {released_at: date} : {}),
    host: `${os.cpus()[0]?.model.trim() ?? 'unknown CPU'}, ${os.cpus().length} logical cores, ${(os.totalmem() / 1024 ** 3).toFixed(1)} GiB RAM`,
    os: `${os.type()} ${os.release()}, ${os.arch()}`,
    rust: rustVerbose.match(/^release: (.+)$/m)?.[1] ?? commandOutput('rustc', ['--version']),
    llvm: rustVerbose.match(/^LLVM version: (.+)$/m)?.[1] ?? 'unknown',
    cargo: commandOutput('cargo', ['--version']).match(/^cargo ([^ ]+)/)?.[1] ?? 'unknown',
    sample_size: Number(sampleSize),
    warmup_seconds: Number(warmupTime),
    measurement_seconds: Number(measurementTime),
  };
  const suffix = kind === 'release' ? version : 'last';
  const file = path.join(reportsDirectory, `${identity.id}-${suffix}.jsonl`);
  writeJsonl(file, runRecord, benchmarks, artifacts);
  console.log(`\nWrote ${path.relative(root, file)} with ${benchmarks.size} benchmarks.`);
}

function loadReports() {
  if (!existsSync(reportsDirectory)) throw new Error('benchmarks/reports does not exist');
  const reports = [];
  for (const name of readdirSync(reportsDirectory).filter((file) => file.endsWith('.jsonl'))) {
    const records = readFileSync(path.join(reportsDirectory, name), 'utf8')
      .split('\n')
      .filter(Boolean)
      .map((line, index) => {
        try {
          return JSON.parse(line);
        } catch (error) {
          throw new Error(`${name}:${index + 1}: ${error.message}`);
        }
      });
    const runRecord = records.find((record) => record.type === 'run');
    if (!runRecord || runRecord.schema_version !== 1) {
      throw new Error(`${name}: missing supported run record`);
    }
    const benchmarks = new Map();
    const artifacts = [];
    for (const record of records) {
      if (record.type === 'benchmark') {
        if (benchmarks.has(record.name)) throw new Error(`${name}: duplicate ${record.name}`);
        benchmarks.set(record.name, record.estimate_ns);
      } else if (record.type === 'artifact') artifacts.push(record);
    }
    if (benchmarks.size === 0) throw new Error(`${name}: no benchmark records`);
    reports.push({file: name, run: runRecord, benchmarks, artifacts});
  }
  return reports;
}

function formatDuration(nanoseconds) {
  if (nanoseconds >= 1_000_000) return `${(nanoseconds / 1_000_000).toFixed(4)} ms`;
  if (nanoseconds >= 1_000) return `${(nanoseconds / 1_000).toFixed(3)} us`;
  return `${nanoseconds.toFixed(2)} ns`;
}

function formatChange(current, previous, language) {
  if (previous === undefined) return language === 'zh' ? '新增' : 'new';
  const change = (current / previous - 1) * 100;
  return `${change > 0 ? '+' : ''}${change.toFixed(1)}%`;
}

function sourceDescription(runRecord, language) {
  if (runRecord.source_kind === 'staged') {
    return language === 'zh'
      ? `staged 源码 \`${runRecord.source_id}\`（基于 \`${runRecord.base_revision}\`）`
      : `staged source \`${runRecord.source_id}\` (base \`${runRecord.base_revision}\`)`;
  }
  if (runRecord.source_kind === 'worktree') {
    return language === 'zh'
      ? `工作区源码 \`${runRecord.source_id}\`（基于 \`${runRecord.base_revision}\`）`
      : `worktree source \`${runRecord.source_id}\` (base \`${runRecord.base_revision}\`)`;
  }
  return `commit \`${runRecord.source_id}\``;
}

function environmentTable(report, language) {
  const runRecord = report.run;
  const zh = language === 'zh';
  const sampling = zh
    ? `${runRecord.sample_size} 个样本，${runRecord.warmup_seconds} 秒预热，${runRecord.measurement_seconds} 秒测量`
    : `${runRecord.sample_size} samples, ${runRecord.warmup_seconds}s warm-up, ${runRecord.measurement_seconds}s measurement`;
  return `| ${zh ? '项目' : 'Item'} | ${zh ? '值' : 'Value'} |
| --- | --- |
| ${zh ? 'Velin 版本' : 'Velin version'} | \`${runRecord.version}\`${runRecord.kind === 'unreleased' ? (zh ? ' 加未发布变更' : ' plus unreleased changes') : ''} |
| ${zh ? '测量源码' : 'Measured source'} | ${sourceDescription(runRecord, language)} |
| ${zh ? '数据文件' : 'Data file'} | \`benchmarks/reports/${report.file}\` |
| ${zh ? '主机' : 'Host'} | ${runRecord.host} |
| ${zh ? '系统' : 'OS'} | ${runRecord.os} |
| Rust | \`rustc ${runRecord.rust}\`, LLVM \`${runRecord.llvm}\` |
| Cargo | \`cargo ${runRecord.cargo}\` |
| ${zh ? '采样' : 'Sampling'} | ${sampling}, plots disabled |
| ${zh ? '日期' : 'Date'} | ${runRecord.date} (Asia/Shanghai) |`;
}

function benchmarkRows(report, comparison, language) {
  return [...report.benchmarks]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([name, current]) => {
      if (!comparison) return `| \`${name}\` | ${formatDuration(current)} |`;
      const previous = comparison.benchmarks.get(name);
      return `| \`${name}\` | ${formatDuration(current)} | ${previous === undefined ? '-' : formatDuration(previous)} | ${formatChange(current, previous, language)} |`;
    })
    .join('\n');
}

const artifactLabels = {
  'runtime-only example': {en: 'Runtime-only example', zh: 'Runtime-only 示例'},
  'C runtime staticlib': {en: 'C runtime static library', zh: 'C runtime 静态库'},
  'runtime-only wasm': {en: 'Runtime-only Wasm', zh: 'Runtime-only Wasm'},
  'source-to-run wasm': {en: 'Source-to-run Wasm', zh: 'Source-to-run Wasm'},
  'full CLI': {en: 'Full CLI', zh: '完整 CLI'},
};
const integer = new Intl.NumberFormat('en-US');

function artifactSection(report, language, headingLevel = 3) {
  if (report.artifacts.length === 0) return '';
  const zh = language === 'zh';
  const rows = report.artifacts.map((artifact) => {
    const label = artifactLabels[artifact.name]?.[language] ?? artifact.name;
    return `| ${label} | ${integer.format(artifact.bytes)} | ${integer.format(artifact.gzip_bytes)} | ${integer.format(artifact.max_bytes)} / ${integer.format(artifact.max_gzip_bytes)} |`;
  }).join('\n');
  return `${'#'.repeat(headingLevel)} ${zh ? '产物体积' : 'Artifact sizes'}

| ${zh ? '产物' : 'Artifact'} | ${zh ? '字节数' : 'Bytes'} | ${zh ? 'Gzip 字节数' : 'Gzip bytes'} | ${zh ? '仓库上限' : 'Checked-in ceiling'} |
| --- | ---: | ---: | ---: |
${rows}`;
}

function latestSection(latest, released, language) {
  const zh = language === 'zh';
  const compared = [...latest.benchmarks.keys()].filter((name) => released.benchmarks.has(name)).length;
  const title = zh
    ? `## 最新未发布报告（${latest.run.date}）`
    : `## Latest unreleased report (${latest.run.date})`;
  const summary = zh
    ? `本报告包含 ${latest.benchmarks.size} 个 benchmark，其中 ${compared} 个同名指标与 ${released.run.date} 发布的 Velin \`${released.run.version}\` 对比。负数表示更快，正数表示更慢；单次本地采样的细小差异不应单独视为性能回退结论。`
    : `This report contains ${latest.benchmarks.size} benchmarks. ${compared} matching metrics are compared with Velin \`${released.run.version}\`, released on ${released.run.date}. Negative changes are faster and positive changes are slower; small differences from one local run are not regression claims.`;
  const headings = zh
    ? `| Benchmark | 当前值 | 已发布 \`${released.run.version}\` | 变化 |`
    : `| Benchmark | Current | Released \`${released.run.version}\` | Change |`;
  return `${title}

${summary}

### ${zh ? '环境与方法' : 'Environment and method'}

${environmentTable(latest, language)}

### ${zh ? 'Benchmark 结果' : 'Benchmark results'}

${headings}
| --- | ---: | ---: | ---: |
${benchmarkRows(latest, released, language)}

${artifactSection(latest, language)}`.trim();
}

function releasedSection(releases, language) {
  const zh = language === 'zh';
  const sections = releases.map((report) => `### Velin \`${report.run.version}\` (${report.run.date})

${zh ? '以下数值来自对应 JSONL，是后续未发布报告的比较基线。' : 'These values come from the corresponding JSONL file and form the comparison baseline for later unreleased reports.'}

${environmentTable(report, language)}

#### ${zh ? 'Benchmark 结果' : 'Benchmark results'}

| Benchmark | Estimate |
| --- | ---: |
${benchmarkRows(report, undefined, language)}

${artifactSection(report, language, 4)}`.trim());
  return `## ${zh ? '已发布快照' : 'Released snapshots'}\n${sections.length ? `\n${sections.join('\n\n')}` : ''}`;
}

function replaceGenerated(file, generated) {
  const markdown = readFileSync(file, 'utf8');
  const start = markdown.indexOf(startMarker);
  const end = markdown.indexOf(endMarker);
  if (start === -1 || end === -1 || end < start) {
    throw new Error(`missing or invalid generated report markers in ${file}`);
  }
  writeFileSync(
    file,
    `${markdown.slice(0, start + startMarker.length)}\n${generated}\n${markdown.slice(end)}`,
  );
}

function generateDocuments() {
  const reports = loadReports();
  const releases = reports
    .filter((report) => report.run.kind === 'release')
    .sort((a, b) => (b.run.released_at ?? b.run.date).localeCompare(a.run.released_at ?? a.run.date));
  const unreleased = reports
    .filter((report) => report.run.kind === 'unreleased')
    .sort((a, b) => b.run.recorded_at.localeCompare(a.run.recorded_at));
  if (releases.length === 0) throw new Error('no released JSONL performance report found');
  if (unreleased.length === 0) throw new Error('no <source>-last.jsonl report found');
  const generated = {
    en: `${latestSection(unreleased[0], releases[0], 'en')}\n\n${releasedSection(releases, 'en')}`,
    zh: `${latestSection(unreleased[0], releases[0], 'zh')}\n\n${releasedSection(releases, 'zh')}`,
  };
  replaceGenerated(docs.en, generated.en);
  replaceGenerated(docs.zh, generated.zh);
  copyFileSync(docs.en, siteDocs.en);
  copyFileSync(docs.zh, siteDocs.zh);
  console.log(
    `Generated documents from ${reports.length} JSONL reports; latest is ${unreleased[0].file}.`,
  );
}

if (!options.generateOnly) recordPerformance();
generateDocuments();
