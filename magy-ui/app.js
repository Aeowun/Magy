// Magy UI - Runtime Integration
const UI = {
    escapeHtml(value) {
        return String(value).replace(/[&<>"']/g, character => ({
            '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#039;'
        }[character]));
    },

    init() {
        this.cacheElements();
        this.bindEvents();
        this.connectSSE();
        this.currentPendingIndex = null;
    },

    cacheElements() {
        this.loadProjectBtn = document.getElementById('load-project-btn');
        this.startupView = document.getElementById('startup-view');
        this.startupActions = document.getElementById('startup-actions');
        this.initZone = document.getElementById('init-zone');
        this.goalInput = document.getElementById('goal-input');
        this.startInitBtn = document.getElementById('start-init-btn');

        this.workspace = document.getElementById('workspace');
        this.projectName = document.getElementById('project-name');
        this.projectGoal = document.getElementById('project-goal');
        this.agentStatus = document.getElementById('agent-status');
        this.taskList = document.getElementById('task-list');
        this.feedContainer = document.getElementById('feed-container');
        this.chatFeed = document.getElementById('chat-feed');
        this.chatForm = document.getElementById('chat-form');
        this.chatInput = document.getElementById('chat-input');
        this.clearFeedBtn = document.getElementById('clear-feed-btn');
        this.focusChatBtn = document.getElementById('focus-chat-btn');
        this.runSummary = document.getElementById('run-summary');
        this.stepSummary = document.getElementById('step-summary');
        this.verifySummary = document.getElementById('verify-summary');
        this.taskCount = document.getElementById('task-count');
        this.activityMode = document.getElementById('activity-mode');
        this.overviewProjectName = document.getElementById('overview-project-name');
        this.overviewGoal = document.getElementById('overview-goal');
        this.overviewTask = document.getElementById('overview-task');
        this.overviewRepo = document.getElementById('overview-repo');
        this.overviewBranch = document.getElementById('overview-branch');
        this.overviewRunState = document.getElementById('overview-run-state');
        this.overviewEvidence = document.getElementById('overview-evidence');
        this.panelDrawer = document.getElementById('panel-drawer');
        this.drawerTitle = document.getElementById('drawer-title');
        this.drawerContent = document.getElementById('drawer-content');
        this.inspector = document.getElementById('inspector');
        this.inspectorContent = document.getElementById('inspector-content');
        this.githubRepoName = document.getElementById('github-repo-name');
        this.githubBranch = document.getElementById('github-branch');
        this.githubStatus = document.getElementById('sc-status');
        this.refreshGithubBtn = document.getElementById('refresh-github-btn');
        this.openGithubBtn = document.getElementById('open-github-btn');

        this.interactionZone = document.getElementById('interaction-zone');
        this.pendingToolRequest = document.getElementById('pending-tool-request');
        this.approveBtn = document.getElementById('approve-btn');
        this.denyBtn = document.getElementById('deny-btn');
        this.autoApproveTools = document.getElementById('auto-approve-tools');

        this.progressBar = document.getElementById('execution-progress');
    },

    bindEvents() {
        this.loadProjectBtn.addEventListener('click', () => this.loadProject());
        this.startInitBtn.addEventListener('click', () => this.initializeProject());
        this.approveBtn.addEventListener('click', () => this.resolveAction(true));
        this.denyBtn.addEventListener('click', () => this.resolveAction(false));
        this.autoApproveTools.addEventListener('change', () => this.updateSettings());
        this.chatForm.addEventListener('submit', (event) => {
            event.preventDefault();
            this.sendChat();
        });
        this.clearFeedBtn.addEventListener('click', () => {
            this.feedContainer.innerHTML = '<div class="empty-feed"><div class="empty-icon">✦</div><h2>Activity cleared</h2><p>New actions and messages will appear here.</p></div>';
        });
        document.querySelectorAll('.rail-button').forEach(button => button.addEventListener('click', () => this.showPanel(button.dataset.panel)));
        document.querySelectorAll('[data-panel-target]').forEach(button => button.addEventListener('click', () => this.showPanel(button.dataset.panelTarget)));
        document.getElementById('close-drawer-btn').addEventListener('click', () => this.panelDrawer.classList.remove('open'));
        document.getElementById('toggle-inspector-btn').addEventListener('click', () => this.inspector.classList.toggle('hidden'));
        document.getElementById('close-inspector-btn').addEventListener('click', () => this.inspector.classList.add('hidden'));
        document.getElementById('overview-run-btn').addEventListener('click', () => this.runAgent());
        this.focusChatBtn.addEventListener('click', () => this.chatInput.focus());
        document.querySelectorAll('.quick-prompt').forEach(button => {
            button.addEventListener('click', () => {
                this.chatInput.value = button.dataset.prompt;
                this.chatInput.focus();
                this.resizeComposer();
            });
        });
        this.chatInput.addEventListener('input', () => this.resizeComposer());
        this.chatInput.addEventListener('keydown', (event) => {
            if (event.key === 'Enter' && !event.shiftKey) {
                event.preventDefault();
                this.chatForm.requestSubmit();
            }
        });
        this.refreshGithubBtn.addEventListener('click', () => this.loadGithubInfo());
        this.openGithubBtn.addEventListener('click', () => {
            if (this.githubUrl) window.open(this.githubUrl, '_blank', 'noopener,noreferrer');
        });
    },

    async secureFetch(url, options = {}) {
        try {
            const resp = await fetch(url, options);
            if (!resp.ok) {
                const text = await resp.text();
                throw new Error(`Server returned ${resp.status}: ${text}`);
            }
            return await resp.json();
        } catch (err) {
            console.error('Fetch Error:', err);
            this.showError("Communication Error", err.message);
            throw err;
        }
    },

    showError(title, message) {
        this.showToast(`${title}: ${message}`, true);
        this.agentStatus.textContent = 'Error';
        this.agentStatus.className = 'status-badge fail';
        this.progressBar.classList.add('hidden');
    },

    async loadProject() {
        const data = await this.secureFetch('/api/load-project', { method: 'POST' });

        if (data.status === 'success') {
            this.showWorkspace(data.project);
        } else if (data.status === 'error' && data.message.includes('FileNotFound')) {
            this.showInitZone();
        } else if (data.status === 'error') {
            this.showError("Project Error", data.message);
        }
    },

    showInitZone() {
        this.startupActions.classList.add('hidden');
        this.initZone.classList.remove('hidden');
    },

    async initializeProject() {
        const goal = this.goalInput.value;
        if (!goal) return alert("Please enter a goal");

        this.startInitBtn.disabled = true;
        this.startInitBtn.textContent = "Generating Project...";

        try {
            const data = await this.secureFetch('/api/initialize', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ goal })
            });
            if (data.status === 'success') {
                this.showWorkspace(data.project);
            }
        } catch (err) {
            this.startInitBtn.disabled = false;
            this.startInitBtn.textContent = "Initialize Project";
        }
    },

    showWorkspace(project) {
        this.startupView.classList.add('hidden');
        this.workspace.classList.remove('hidden');
        this.updateProjectUI(project);
        this.showPanel('overview');
        this.loadGithubInfo();
        this.runAgent();
    },

    async loadGithubInfo() {
        try {
            const data = await this.secureFetch('/api/github-info');
            if (data.status !== 'success') return;
            const info = data.github;
            this.githubUrl = info.github_url;
            this.githubRepoName.textContent = info.github_url
                ? info.github_url.replace('https://github.com/', '')
                : (info.is_git_repository ? 'Local Git repository' : 'Not a Git repository');
            this.githubBranch.textContent = info.branch
                ? `${info.branch} · ${info.changed_files} changed`
                : 'No branch detected';
            this.githubStatus.classList.toggle('connected', Boolean(info.github_url));
            this.openGithubBtn.classList.toggle('hidden', !info.github_url);
            this.overviewRepo.textContent = this.githubRepoName.textContent;
            this.overviewBranch.textContent = this.githubBranch.textContent;
        } catch (err) {
            this.githubRepoName.textContent = 'Repository unavailable';
            this.githubBranch.textContent = 'Could not inspect Git metadata';
        }
    },

    updateProjectUI(project) {
        this.projectName.textContent = project.name;
        this.projectGoal.textContent = project.goal;
        this.overviewProjectName.textContent = project.name;
        this.overviewGoal.textContent = project.goal;
        this.renderTaskList(project.tasks);
        this.taskCount.textContent = project.tasks.length;
        const active = project.tasks.find(task => task.status.toLowerCase() === 'in_progress') || project.tasks.find(task => task.status.toLowerCase() !== 'done');
        this.overviewTask.textContent = active ? active.description : 'All tasks complete';
    },

    renderTaskList(tasks) {
        this.taskList.innerHTML = tasks.map(task => `
            <li class="task-item ${task.status.toLowerCase()}">
                <div class="task-checkbox"></div>
                <div class="task-desc">${task.description}</div>
            </li>
        `).join('');
    },

    async runAgent() {
        this.progressBar.classList.remove('hidden');
        this.agentStatus.textContent = 'Executing';
        this.agentStatus.className = 'status-badge active';
        this.runSummary.textContent = 'Working through the active task…';
        this.overviewRunState.textContent = 'Running';
        try {
            const result = await this.secureFetch('/api/run', { method: 'POST' });
            if (result.status === 'error') {
                this.showError("Agent Error", result.message);
            }
        } catch (err) {
            this.agentStatus.textContent = 'Error';
        }
    },

    async updateSettings() {
        try {
            await this.secureFetch('/api/settings', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ auto_approve_tools: this.autoApproveTools.checked })
            });
            this.agentStatus.textContent = this.autoApproveTools.checked
                ? 'Auto-execute enabled'
                : 'Approval required';
        } catch (err) {
            this.autoApproveTools.checked = !this.autoApproveTools.checked;
        }
    },

    async resolveAction(approved) {
        this.interactionZone.classList.add('hidden');
        try {
            this.agentStatus.textContent = approved ? 'Approved' : 'Denied';
            const result = await this.secureFetch('/api/resolve', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ index: this.currentPendingIndex, approved })
            });
            if (result.status === 'success') {
                this.runAgent(); // Resume only after the action was actually resolved.
            }
        } catch (err) {
            this.showError("Approval Error", err.message);
        }
    },

    connectSSE() {
        const events = new EventSource('/api/events');
        events.onmessage = (e) => {
            try {
                const event = JSON.parse(e.data);
                this.handleBackendEvent(event);
            } catch (err) {
                // Ignore pings
            }
        };
        events.onerror = (e) => {
            console.error('SSE Error:', e);
            // Reconnection happens automatically, but we can log it.
        };
    },

    handleBackendEvent(event) {
        console.log('Backend Event:', event);
        switch (event.type) {
            case 'ProjectLoaded':
                this.updateProjectUI(event.data);
                break;
            case 'AgentStateChanged':
                this.agentStatus.textContent = event.data;
                break;
            case 'Step':
                this.appendActivity(event.data.step, event.data.index);
                this.stepSummary.textContent = `${event.data.index + 1} steps`;
                break;
            case 'Stop':
                this.agentStatus.textContent = 'Idle';
                this.agentStatus.className = 'status-badge';
                this.runSummary.textContent = event.data === 'Task completed successfully' ? 'Task completed' : event.data;
                this.overviewRunState.textContent = event.data === 'Task completed successfully' ? 'Completed' : 'Stopped';
                this.progressBar.classList.add('hidden');
                if (event.data === 'Action requires approval') {
                    this.showApproval();
                } else if (event.data && event.data.startsWith('Verification warning')) {
                    this.appendWarning(event.data);
                } else if (event.data && event.data !== 'Task completed successfully') {
                    this.appendSystemMessage(event.data);
                }
                break;
            case 'Error':
                this.showError("Runtime Error", event.data);
                break;
            case 'Warning':
                this.appendWarning(event.data);
                break;
            case 'Chat':
                this.appendChat(event.data.role, event.data.content);
                break;
        }
    },

    appendSystemMessage(message) {
        const element = document.createElement('div');
        element.className = 'activity-item system-message';
        element.textContent = `Magy stopped: ${message}`;
        this.feedContainer.appendChild(element);
        element.scrollIntoView({ behavior: 'smooth' });
    },

    appendWarning(message) {
        const element = document.createElement('div');
        element.className = 'activity-item warning-message';
        element.textContent = `Warning: ${message}`;
        this.feedContainer.appendChild(element);
        element.scrollIntoView({ behavior: 'smooth' });
    },

    appendChat(role, content) {
        const element = document.createElement('div');
        element.className = `chat-message ${role}`;
        element.textContent = content;
        this.chatFeed.appendChild(element);
        element.scrollIntoView({ behavior: 'smooth' });
    },

    showPanel(panel) {
        document.querySelectorAll('.workbench-panel').forEach(element => element.classList.remove('active-panel'));
        const target = document.getElementById(`${panel}-panel`);
        if (target) target.classList.add('active-panel');
        document.querySelectorAll('.rail-button').forEach(button => button.classList.toggle('active', button.dataset.panel === panel));
        if (['tasks', 'source', 'settings'].includes(panel)) {
            this.panelDrawer.classList.remove('open');
        }
    },

    resizeComposer() {
        this.chatInput.style.height = 'auto';
        this.chatInput.style.height = `${Math.min(this.chatInput.scrollHeight, 140)}px`;
    },

    showToast(message, isError = false) {
        const toast = document.createElement('div');
        toast.className = `toast ${isError ? 'error' : ''}`;
        toast.textContent = message;
        document.body.appendChild(toast);
        setTimeout(() => toast.remove(), 4500);
    },

    async sendChat() {
        const message = this.chatInput.value.trim();
        if (!message) return;
        this.chatInput.value = '';
        await this.secureFetch('/api/chat', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ message })
        });
    },

    appendActivity(step, index) {
        // Avoid duplicate steps (simplification)
        const existing = document.getElementById(`step-${index}`);
        if (existing) return;

        const element = document.createElement('div');
        element.className = 'activity-item';
        element.id = `step-${index}`;

        let html = `
            <div class="reasoning-block">
                <div class="reasoning-label">Reasoning</div>
                <div class="reasoning-content">${this.escapeHtml(step.model_response.content)}</div>
            </div>
        `;

        if (step.action_record) {
            const record = step.action_record;
            this.lastRecord = record;
            this.lastIndex = index;
            html += `
                <div class="tool-call">
                    <span class="tool-label">Tool Call: ${this.escapeHtml(record.request.tool)}</span>
                    <pre style="margin: 0; font-size: 0.8rem;">${this.escapeHtml(JSON.stringify(record.request, null, 2))}</pre>
                    <div style="margin-top: 0.5rem; color: #94a3b8; font-size: 0.8rem;">Outcome: ${this.escapeHtml(JSON.stringify(record.outcome))}</div>
                </div>
            `;
        }

        if (step.verification) {
            const ver = step.verification;
            html += `
                <div class="verification-block ${ver.passed ? 'pass' : 'warn'}">
                    <span style="font-weight: bold;">Verification: ${ver.passed ? 'PASSED' : 'WARNING — CHECK FAILED'}</span>
                    <pre style="font-size: 0.7rem; margin-top: 0.5rem; white-space: pre-wrap;">${this.escapeHtml(ver.command)}\n${this.escapeHtml(ver.stdout)}${this.escapeHtml(ver.stderr)}</pre>
                </div>
            `;
        }

        element.innerHTML = html;
        element.addEventListener('click', () => {
            this.inspector.classList.remove('hidden');
            this.inspectorContent.innerHTML = `<div class="inspector-card"><span class="eyebrow">STEP ${index + 1}</span><h3>${this.escapeHtml(record ? record.request.tool : 'Model response')}</h3><p class="muted-text">Expand the activity card in Activity to review the full reasoning, request, and outcome.</p></div>`;
        });
        this.feedContainer.appendChild(element);
        element.scrollIntoView({ behavior: 'smooth' });
    },

    showApproval() {
        this.pendingToolRequest.textContent = JSON.stringify(this.lastRecord.request, null, 2);
        this.currentPendingIndex = this.lastIndex;
        this.interactionZone.classList.remove('hidden');
    }
};

document.addEventListener('DOMContentLoaded', () => UI.init());
