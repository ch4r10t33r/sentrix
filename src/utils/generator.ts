import fs from 'fs-extra';
import path from 'path';

/**
 * Copy a template folder to the destination, optionally replacing
 * {{AGENT_NAME}}, {{PROJECT_NAME}}, {{CAPABILITIES}} tokens in every file.
 */
export async function copyTemplate(
  templateDir: string,
  destDir: string,
  tokens: Record<string, string> = {}
): Promise<void> {
  await fs.ensureDir(destDir);
  await fs.copy(templateDir, destDir, { overwrite: false });
  await replaceTokensInDir(destDir, tokens);
}

async function replaceTokensInDir(
  dir: string,
  tokens: Record<string, string>
): Promise<void> {
  const entries = await fs.readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      await replaceTokensInDir(fullPath, tokens);
    } else {
      let content = await fs.readFile(fullPath, 'utf8');
      for (const [key, value] of Object.entries(tokens)) {
        content = content.replaceAll(`{{${key}}}`, value);
      }
      await fs.writeFile(fullPath, content, 'utf8');
    }
  }
}

/**
 * Write a borgkit.config.json to the project root.
 */
export async function writeConfig(
  projectDir: string,
  config: Record<string, unknown>
): Promise<void> {
  const configPath = path.join(projectDir, 'borgkit.config.json');
  await fs.writeJson(configPath, config, { spaces: 2 });
}
