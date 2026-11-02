#!/usr/bin/env node
/**
 * dsh-mirsa setup: turn a fresh machine into a working MIRSA + DSH plugin.
 *
 * Steps: place the vendored MIRSA source, install the rust nightly toolchain
 * (rustc-dev/rust-src), make sure z3 and Python >= 3.10 exist, then hand over
 * to mac-port/setup-macos.py which builds the worker and the MCP venv.
 */
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import { homedir, platform } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const PKG = resolve(fileURLToPath(new URL('..', import.meta.url)));
const TOOLCHAIN = 'nightly-2025-02-14';

function log(step, message) {
  process.stdout.write(`\n[mirsa] ${step}: ${message}\n`);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: 'inherit', ...options });
  if (result.error) throw new Error(`${command} 启动失败：${result.error.message}`);
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} 退出码 ${result.status}`);
}

function capture(command, args) {
  const result = spawnSync(command, args, { encoding: 'utf8' });
  if (result.error) throw new Error(`${command} 启动失败：${result.error.message}`);
  return result.status === 0 ? result.stdout.trim() : null;
}

function which(binary) {
  return capture('bash', ['-lc', `command -v ${binary}`]);
}

function hasPython310() {
  for (const name of ['python3.13', 'python3.12', 'python3.11', 'python3.10', 'python3']) {
    const path = which(name);
    if (!path) continue;
    const version = capture(path, ['-c', 'import sys;print("%d.%d" % sys.version_info[:2])']);
    if (version && Number(version.split('.')[0]) === 3 && Number(version.split('.')[1]) >= 10) {
      return { path, version };
    }
  }
  return null;
}

function pickPipIndex() {
  if (process.env.PIP_INDEX_URL) return process.env.PIP_INDEX_URL;
  const probe = spawnSync('curl', ['-sS', '-o', '/dev/null', '-m', '8', 'https://pypi.org/simple/']);
  if (!probe.error && probe.status === 0) return null;
  // PyPI unreachable from here (常见于 IPv6 不通的网络)；退回国内镜像。
  const mirror = 'https://pypi.tuna.tsinghua.edu.cn/simple';
  process.stdout.write(`\n[mirsa] PyPI 直连失败，改用镜像 ${mirror}\n`);
  return mirror;
}

function parseArgs(argv) {
  const options = { root: process.env.MIRSA_ROOT || join(homedir(), 'mirsa') };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--root') options.root = resolve(argv[++i]);
    else if (arg === '--z3-prefix') options.z3 = resolve(argv[++i]);
    else if (arg === '--python') options.python = argv[++i];
    else if (arg === '--skip-toolchain') options.skipToolchain = true;
    else if (arg === '--help' || arg === '-h') options.help = true;
    else throw new Error(`未知参数：${arg}`);
  }
  return options;
}

function installSource(root) {
  const marker = join(root, 'Cargo.toml');
  if (existsSync(marker)) {
    log('源码', `${root} 已存在，跳过解包`);
    return;
  }
  const tarball = join(PKG, 'vendor', 'mirsa-src.tar.gz');
  if (!existsSync(tarball)) throw new Error(`找不到内置源码包：${tarball}`);
  mkdirSync(root, { recursive: true });
  log('源码', `解包到 ${root}`);
  run('tar', ['-xzf', tarball, '-C', root]);
}

function toolchainReady() {
  const version = capture('rustup', ['run', TOOLCHAIN, 'rustc', '--version']);
  return Boolean(version && version.startsWith('rustc'));
}

function installToolchain() {
  if (toolchainReady()) {
    log('工具链', `${TOOLCHAIN} 已安装，跳过`);
    return;
  }
  if (!which('rustup')) {
    log('工具链', '安装 rustup');
    run('bash', ['-lc', 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal']);
  }
  log('工具链', `${TOOLCHAIN} + rustc-dev + rust-src`);
  run('bash', ['-lc', `rustup toolchain install ${TOOLCHAIN} --profile minimal --component rustc-dev,rust-src`]);
}

function findZ3(root) {
  const candidates = [
    process.env.Z3_PREFIX,
    join(root, 'third-party', 'z3'),
    '/opt/homebrew/opt/z3',
    '/usr/local/opt/z3',
  ].filter(Boolean);
  for (const base of candidates) {
    if (existsSync(join(base, 'include', 'z3.h'))) {
      const libdir = existsSync(join(base, 'lib')) ? join(base, 'lib') : join(base, 'bin');
      return { prefix: base, libdir };
    }
  }
  return null;
}

function installZ3(root) {
  const found = findZ3(root);
  if (found) {
    log('z3', `已找到 ${found.prefix}`);
    return found.prefix;
  }
  const bundled = join(PKG, 'vendor', 'z3.tar.gz');
  if (existsSync(bundled)) {
    log('z3', `使用包内置的 z3，解包到 ${join(root, 'third-party', 'z3')}`);
    mkdirSync(join(root, 'third-party', 'z3'), { recursive: true });
    run('tar', ['-xzf', bundled, '-C', join(root, 'third-party', 'z3')]);
    const extracted = findZ3(root);
    if (extracted) return extracted.prefix;
  }
  const brew = which('brew');
  if (brew) {
    log('z3', 'brew install z3');
    run(brew, ['install', 'z3']);
    const installed = findZ3(root);
    if (installed) return installed.prefix;
  }
  throw new Error('找不到 z3。请先 `brew install z3`，或用 --z3-prefix 指定官方包目录');
}

function installPython(root) {
  const python = hasPython310();
  if (python) {
    log('Python', `${python.version} (${python.path})`);
    return python.path;
  }
  const bundled = join(PKG, 'vendor', 'python.tar.gz');
  if (existsSync(bundled)) {
    log('Python', `使用包内置的 Python，解包到 ${join(root, 'deps', 'python')}`);
    mkdirSync(join(root, 'third-party'), { recursive: true });
    run('tar', ['-xzf', bundled, '-C', join(root, 'third-party')]);
    const extracted = join(root, 'third-party', 'python', 'bin', 'python3');
    if (existsSync(extracted)) {
      const version = capture(extracted, ['-c', 'import sys;print("%d.%d" % sys.version_info[:2])']);
      log('Python', `${version} (${extracted})`);
      return extracted;
    }
  }
  const brew = which('brew');
  if (!brew) throw new Error('需要 Python >= 3.10（mcp 2.3.0 的要求）；请先安装后重试');
  log('Python', 'brew install python@3.12');
  run(brew, ['install', 'python@3.12']);
  const installed = hasPython310();
  if (!installed) throw new Error('Python 安装后仍未找到 >= 3.10');
  return installed.path;
}

function main() {
  const argv = process.argv.slice(2);
  // `dsh-mirsa setup` is the documented form; bare flags also work.
  if (argv[0] === 'setup') argv.shift();
  const options = parseArgs(argv);
  if (options.help) {
    process.stdout.write([
      '用法: npx dsh-mirsa setup [选项]',
      '',
      '  --root <dir>         MIRSA 安装目录（默认 $MIRSA_ROOT 或 ~/mirsa）',
      '  --z3-prefix <dir>    指定 z3 目录（含 include/z3.h）',
      '  --python <path>      venv 使用的 Python >= 3.10',
      '  --skip-toolchain     跳过 rustup 工具链安装（已装好时用）',
      '',
    ].join('\n'));
    return;
  }
  if (platform() !== 'darwin' && platform() !== 'linux') {
    throw new Error(`暂不支持 ${platform()}；目前只验证了 macOS`);
  }
  log('目标', options.root);
  installSource(options.root);
  if (!options.skipToolchain) installToolchain();
  const z3 = options.z3 || installZ3(options.root);
  const python = options.python || installPython(options.root);
  const setup = join(options.root, 'scripts', 'setup.py');
  if (!existsSync(setup)) throw new Error(`源码不完整，缺少 ${setup}`);
  log('构建', '调用 mac-port/setup-macos.py（首次编译较慢）');
  const args = [setup, '--root', options.root];
  if (z3) args.push('--z3-prefix', z3);
  if (python) args.push('--python', python);
  const index = pickPipIndex();
  run(python, args, index ? { env: { ...process.env, PIP_INDEX_URL: index } } : {});
  process.stdout.write([
    '',
    '[mirsa] 完成。',
    `  MIRSA_ROOT=${options.root}`,
    '  重启 DSH 后，mirsa_query 工具与 mirsa-unsafe-check skill 即可使用。',
    '',
  ].join('\n'));
}

try {
  main();
} catch (error) {
  process.stderr.write(`\n[mirsa] 失败：${error.message}\n`);
  process.exit(1);
}
