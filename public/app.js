const API = '/api';

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

  memories.forEach(mem => {
    const card = document.createElement('div');
    card.className = 'memory-card';
    card.dataset.id = mem.id;
    card.innerHTML = `
      <div class="memory-header">
        <span class="type type-${mem.memory_type}">${mem.memory_type}</span>
        <span class="project">${mem.project || 'global'}</span>
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

// Event listeners
document.addEventListener('DOMContentLoaded', () => {
  loadProjects();
  loadMemories();

  document.getElementById('memoriesList').addEventListener('click', (e) => {
    if (e.target.classList.contains('delete-btn')) {
      const id = e.target.dataset.id;
      const project = e.target.dataset.project;
      deleteMemory(id, project);
    }
  });

  document.getElementById('searchBtn').addEventListener('click', () => {
    const query = document.getElementById('searchBox').value;
    const project = document.getElementById('projectFilter').value;
    loadMemories(project || null, query || null);
  });

  document.getElementById('searchBox').addEventListener('keypress', (e) => {
    if (e.key === 'Enter') {
      document.getElementById('searchBtn').click();
    }
  });

  document.getElementById('projectFilter').addEventListener('change', () => {
    const project = document.getElementById('projectFilter').value;
    loadMemories(project || null, null);
  });
});
