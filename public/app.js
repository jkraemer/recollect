const API = '/api';

let vectorsEnabled = false;

async function loadVectorStatus(project = null) {
  try {
    const params = new URLSearchParams();
    if (project !== null) params.append('project', project);

    const resp = await fetch(`${API}/vectors/status?${params}`);
    const data = await resp.json();
    const statusEl = document.getElementById('embeddingStatus');

    if (data.enabled) {
      vectorsEnabled = true;
      if (data.pending > 0) {
        statusEl.innerHTML = `⏳ <span class="count">${data.pending}</span> memories pending embedding`;
        statusEl.style.display = 'block';
      } else {
        statusEl.style.display = 'none';
      }
    } else {
      vectorsEnabled = false;
      statusEl.style.display = 'none';
    }
  } catch (err) {
    console.error('Failed to load vector status:', err);
  }
}

async function loadMemories(project = null, query = null) {
  const params = new URLSearchParams();
  if (project) params.append('project', project);
  if (query) params.append('q', query);

  const endpoint = query
    ? `${API}/memories/search?${params}`
    : `${API}/memories?${params}`;

  try {
    const resp = await fetch(endpoint);
    const data = await resp.json();
    const memories = query ? data.results : data;
    displayMemories(memories);
  } catch (err) {
    console.error('Failed to load memories:', err);
  }
}

function displayMemories(memories) {
  const container = document.getElementById('memoriesList');
  container.innerHTML = '';

  if (!memories || memories.length === 0) {
    container.innerHTML = '<p class="empty">No memories found</p>';
    return;
  }

  const currentProject = getSelectedProject();
  const showProjectLinks = currentProject === '__all__';

  memories.forEach(mem => {
    const card = document.createElement('div');
    card.className = 'memory-card';
    card.dataset.id = mem.id;

    // Show embedding indicator only when vectors enabled and embedding is missing
    const embeddingIndicator = (vectorsEnabled && mem.has_embedding === false)
      ? '<span class="embedding-pending" title="Pending embedding">⏳</span>'
      : '';

    // Show project as link in "All Projects" view, hide in project views
    let projectHtml = '';
    if (showProjectLinks) {
      const projectName = mem.project || 'global';
      const projectValue = mem.project || '';
      projectHtml = `<a href="#" class="project-link" data-project="${projectValue}">${projectName}</a>`;
    }

    card.innerHTML = `
      <div class="memory-header">
        <span class="type type-${mem.memory_type}">${mem.memory_type}</span>
        ${projectHtml}
        ${embeddingIndicator}
        <span class="date">${formatDate(mem.created_at)}</span>
      </div>
      <div class="content">${escapeHtml(mem.content)}</div>
      <div class="memory-actions">
        <button class="delete-btn" data-id="${mem.id}" data-project="${mem.project || ''}">Delete</button>
      </div>
    `;
    container.appendChild(card);
  });
}

async function loadProjects() {
  try {
    const resp = await fetch(`${API}/projects`);
    const data = await resp.json();
    const select = document.getElementById('projectFilter');

    data.projects.forEach(proj => {
      const option = document.createElement('option');
      option.value = proj;
      option.textContent = proj;
      select.appendChild(option);
    });
  } catch (err) {
    console.error('Failed to load projects:', err);
  }
}

function formatDate(iso) {
  return new Date(iso).toLocaleDateString();
}

function escapeHtml(text) {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

async function deleteMemory(id, project) {
  const params = project ? `?project=${encodeURIComponent(project)}` : '';
  try {
    const resp = await fetch(`${API}/memories/${id}${params}`, { method: 'DELETE' });
    if (resp.ok) {
      document.querySelector(`.memory-card[data-id="${id}"]`)?.remove();
    }
  } catch (err) {
    console.error('Failed to delete memory:', err);
  }
}

function getProjectFromPath() {
  const match = window.location.pathname.match(/^\/projects\/(.+)$/);
  return match ? decodeURIComponent(match[1]) : null;
}

function navigateToProject(project) {
  const path = project ? `/projects/${encodeURIComponent(project)}` : '/';
  window.history.replaceState({}, '', path);
}

function getSelectedProject() {
  const value = document.getElementById('projectFilter').value;
  return value || null;  // "" (global) becomes null, "__all__" stays "__all__"
}

// Event listeners
document.addEventListener('DOMContentLoaded', async () => {
  await loadProjects();

  // Restore project from URL path
  const savedProject = getProjectFromPath();
  const projectFilter = document.getElementById('projectFilter');
  if (savedProject && [...projectFilter.options].some(o => o.value === savedProject)) {
    projectFilter.value = savedProject;
  }

  const project = getSelectedProject();
  await loadVectorStatus(project);
  loadMemories(project);

  document.getElementById('memoriesList').addEventListener('click', (e) => {
    if (e.target.classList.contains('delete-btn')) {
      const id = e.target.dataset.id;
      const project = e.target.dataset.project;
      deleteMemory(id, project);
    } else if (e.target.classList.contains('project-link')) {
      e.preventDefault();
      const project = e.target.dataset.project;
      document.getElementById('projectFilter').value = project;
      navigateToProject(project);
      loadVectorStatus(project || null);
      loadMemories(project || null, null);
    }
  });

  document.getElementById('searchBtn').addEventListener('click', () => {
    const query = document.getElementById('searchBox').value;
    const project = getSelectedProject();
    loadMemories(project, query || null);
  });

  document.getElementById('searchBox').addEventListener('keypress', (e) => {
    if (e.key === 'Enter') {
      document.getElementById('searchBtn').click();
    }
  });

  document.getElementById('projectFilter').addEventListener('change', () => {
    const project = getSelectedProject();
    navigateToProject(project);
    loadVectorStatus(project);
    loadMemories(project, null);
  });
});
