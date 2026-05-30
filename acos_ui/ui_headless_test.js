import { chromium } from 'playwright';
import { spawn } from 'child_process';
import { join } from 'path';

const UI_DIR = '/home/ankheru/Documents/Projects/ACOS/acos_ui';
const SCREENSHOT_PATH = '/home/ankheru/.gemini/antigravity-cli/brain/a382703b-b213-4d1f-872d-115670a6fb4c/acos_ui_headless.png';

async function runTest() {
  console.log('[*] Démarrage du serveur de développement Vite...');
  
  const viteProcess = spawn('npm', ['run', 'dev'], {
    cwd: UI_DIR,
    shell: true
  });

  viteProcess.stdout.on('data', (data) => {
    // console.log(`[Vite stdout] ${data}`);
  });

  viteProcess.stderr.on('data', (data) => {
    console.error(`[Vite stderr] ${data}`);
  });

  // Laisser 2 secondes à Vite pour s'initialiser
  await new Promise((resolve) => setTimeout(resolve, 2500));

  console.log('[*] Lancement de Chromium Headless...');
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();

  const consoleErrors = [];
  const allConsoleLogs = [];

  // Intercepter les erreurs de la console
  page.on('pageerror', (err) => {
    console.error(`[Browser PageError] ${err.toString()}`);
    consoleErrors.push({ type: 'pageerror', message: err.toString() });
  });

  page.on('console', (msg) => {
    const text = msg.text();
    allConsoleLogs.push({ type: msg.type(), text: text });
    if (msg.type() === 'error') {
      console.error(`[Browser Console Error] ${text}`);
      consoleErrors.push({ type: 'console-error', message: text });
    } else {
      console.log(`[Browser Console ${msg.type()}] ${text}`);
    }
  });

  try {
    console.log('[*] Navigation vers http://localhost:5173/...');
    await page.goto('http://localhost:5173/', { waitUntil: 'networkidle', timeout: 5000 });
    
    console.log('[*] Attente du rendu visuel de la Bento Grid...');
    await page.waitForTimeout(2000); // Temps pour Framer Motion d'animer les entrées

    // Prendre une capture d'écran pour vérification visuelle
    console.log(`[*] Capture d'écran enregistrée dans : ${SCREENSHOT_PATH}`);
    await page.screenshot({ path: SCREENSHOT_PATH, fullPage: true });

    // Tenter d'interagir légèrement pour voir s'il y a des bogues de saisie
    console.log('[*] Test de saisie dans le terminal interactif...');
    await page.fill('.terminal-input', 'ls -la');
    
    // Attendre un peu
    await page.waitForTimeout(1000);

    // Faire une deuxième capture après interaction
    await page.screenshot({ path: SCREENSHOT_PATH, fullPage: true });

  } catch (err) {
    console.error(`[FAIL] Erreur de navigation : ${err.message}`);
    consoleErrors.push({ type: 'navigation-error', message: err.message });
  } finally {
    console.log('[*] Fermeture du navigateur et du serveur Vite...');
    await browser.close();
    
    try {
      viteProcess.kill('SIGINT');
    } catch (e) {}
  }

  // Renvoyer le bilan des erreurs
  console.log('\n================ BILAN DU TEST HEADLESS ================');
  console.log(`Erreurs détectées : ${consoleErrors.length}`);
  if (consoleErrors.length > 0) {
    console.log('Liste des erreurs :');
    consoleErrors.forEach((e, idx) => {
      console.log(`  ${idx + 1}. [${e.type}] ${e.message}`);
    });
  } else {
    console.log('✓ Aucune erreur Javascript détectée dans la console Chromium !');
  }
  console.log('========================================================');
  
  process.exit(consoleErrors.length > 0 ? 1 : 0);
}

runTest().catch((err) => {
  console.error('Test script crashed:', err);
  process.exit(1);
});
