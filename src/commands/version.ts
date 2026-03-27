import { parseVersion, VERSION } from '../version';
import { logger } from '../utils/logger';
import chalk from 'chalk';

export function versionCommand(): void {
  const info = parseVersion(VERSION);

  logger.title('Inai CLI');
  console.log(`  Version    : ${chalk.green(info.version)}`);
  console.log(`  Major      : ${info.major}`);
  console.log(`  Minor      : ${info.minor}`);
  console.log(`  Patch      : ${info.patch}`);
  if (info.prerelease) {
    console.log(`  Pre-release: ${chalk.yellow(info.prerelease)}`);
  }
  console.log(`  Build date : ${info.buildDate}`);
  console.log(`  Node.js    : ${process.version}`);
  console.log(`  Platform   : ${process.platform} ${process.arch}`);
  console.log('');
  logger.dim('  Docs  : https://github.com/ch4r10t33r/inai/tree/main/docs');
  logger.dim('  Issues: https://github.com/ch4r10t33r/inai/issues');
  console.log('');
}
