// About Dialog
import { openDialog, closeDialog } from './dialog.js';
export function setupAboutDialog() {
  const aboutLink = document.getElementById('about-link');
  const closeAboutBtn = document.getElementById('close-about');

  if (aboutLink) {
    aboutLink.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      showAbout();
    });
  }

  if (closeAboutBtn) {
    closeAboutBtn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      closeAbout();
    });
  }
}

function showAbout() {
  console.log('[About] Opening dialog');
  openDialog('about-dialog');
  setupAboutLinks();
}

function closeAbout() {
  closeDialog('about-dialog');
}

const { invoke } = window.__TAURI__.core;

function setupAboutLinks() {
  const githubLink = document.getElementById('link-github');
  const docsLink = document.getElementById('link-docs');

  if (githubLink && !githubLink.hasAttribute('data-listener')) {
    githubLink.setAttribute('data-listener', 'true');
    githubLink.addEventListener('click', async (e) => {
      e.preventDefault();
      try {
        await invoke('open_url', {
          url: 'https://github.com/TysonTranThai/ThinkUtils-Maintained-Upgraded'
        });
      } catch (err) {
        console.error('[About] Failed to open GitHub link:', err);
      }
    });
  }

  if (docsLink && !docsLink.hasAttribute('data-listener')) {
    docsLink.setAttribute('data-listener', 'true');
    docsLink.addEventListener('click', async (e) => {
      e.preventDefault();
      try {
        await invoke('open_url', {
          url: 'https://thinkutils.tysontranthai.site'
        });
      } catch (err) {
        console.error('[About] Failed to open docs link:', err);
      }
    });
  }
}
