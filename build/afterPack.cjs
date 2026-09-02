/**
 * The packager's after-pack hook.
 *
 * An extra-resources filter that matches nothing ships nothing and says nothing. Building for
 * one target over a core built for the other would therefore produce an installer with no core,
 * a failure that otherwise stays invisible until launch.
 *
 * Packing from a filesystem without POSIX modes can also strip the Linux core's execute bit;
 * a core without `+x` is one of the named spawn failures. The hook restores and rechecks it.
 *
 * Finally, a Windows installer without either architecture's Linux worker cannot index those
 * repositories and would fail silently at runtime, so that pack is refused here too.
 */
const { chmodSync, existsSync, statSync } = require('node:fs');
const { join } = require('node:path');

const WORKER_ARCHES = ['x64', 'arm64'];

exports.default = async function afterPack(context) {
  const platform = context.electronPlatformName;
  if (platform !== 'win32' && platform !== 'linux') {
    throw new Error(`afterPack: unsupported target platform ${platform}`);
  }

  const isWindows = platform === 'win32';
  const resources = join(context.appOutDir, 'resources');
  const core = join(resources, 'core', isWindows ? 'codotheca-core.exe' : 'codotheca-core');

  if (!existsSync(core)) {
    throw new Error(
      `afterPack: the core binary was not staged: ${core}\n` +
        'Build the core for this target first (`npm run build:core`).',
    );
  }

  if (!isWindows) {
    chmodSync(core, 0o755);
    if ((statSync(core).mode & 0o111) === 0) {
      throw new Error(`afterPack: the core binary is not executable after chmod: ${core}`);
    }
    return;
  }

  const missingWorkers = WORKER_ARCHES.map((arch) =>
    join(resources, 'worker', `linux-${arch}`, 'codotheca-worker'),
  ).filter((worker) => !existsSync(worker));
  if (missingWorkers.length > 0) {
    throw new Error(
      `afterPack: the WSL worker was not staged:\n${missingWorkers.join('\n')}\n` +
        'Build each missing worker for a Linux target and stage it with ' +
        '`node scripts/stage-worker.mjs --from <path>` (spec §13).',
    );
  }
};
