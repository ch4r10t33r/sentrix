import fs from 'fs-extra';
import path from 'path';

/**
 * Auto-detect the language used in an existing Borgkit project
 * by reading borgkit.config.json or sniffing file extensions.
 */
export function detectLanguage(projectDir: string): string {
  const configPath = path.join(projectDir, 'borgkit.config.json');
  if (fs.existsSync(configPath)) {
    const config = fs.readJsonSync(configPath);
    if (config.lang) return config.lang;
  }
  if (fs.existsSync(path.join(projectDir, 'tsconfig.json')))       return 'typescript';
  if (fs.existsSync(path.join(projectDir, 'requirements.txt')))    return 'python';
  if (fs.existsSync(path.join(projectDir, 'Cargo.toml')))          return 'rust';
  if (fs.existsSync(path.join(projectDir, 'build.zig')))           return 'zig';
  return 'typescript'; // sensible default
}
