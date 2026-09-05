// Magy UI - Runtime Integration
const UI = {
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
        this.chatForm = document.getElementById('chat-form');
        this.chatInput = document.getElementById('chat-input');

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
        alert(`${title}\n\n${message}\n\nPlease check the backend logs for details.`);
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
        this.runAgent();
    },

    updateProjectUI(project) {
        this.projectName.textContent = project.name;
        this.projectGoal.textContent = project.goal;
        this.renderTaskList(project.tasks);
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
        await this.secureFetch('/api/run', { method: 'POST' });
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
        await this.secureFetch('/api/resolve', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ index: this.currentPendingIndex, approved })
        });
        this.runAgent(); // Resume
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
                break;
            case 'Stop':
                this.agentStatus.textContent = 'Idle';
                this.agentStatus.className = 'status-badge';
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
        this.feedContainer.appendChild(element);
        element.scrollIntoView({ behavior: 'smooth' });
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
                <div class="reasoning-content">${step.model_response.content}</div>
            </div>
        `;

        if (step.action_record) {
            const record = step.action_record;
            this.lastRecord = record;
            this.lastIndex = index;
            html += `
                <div class="tool-call">
                    <span class="tool-label">Tool Call: ${record.request.tool}</span>
                    <pre style="margin: 0; font-size: 0.8rem;">${JSON.stringify(record.request, null, 2)}</pre>
                    <div style="margin-top: 0.5rem; color: #94a3b8; font-size: 0.8rem;">Outcome: ${JSON.stringify(record.outcome)}</div>
                </div>
            `;
        }

        if (step.verification) {
            const ver = step.verification;
            html += `
                <div class="verification-block ${ver.passed ? 'pass' : 'warn'}">
                    <span style="font-weight: bold;">Verification: ${ver.passed ? 'PASSED' : 'WARNING — CHECK FAILED'}</span>
                    <pre style="font-size: 0.7rem; margin-top: 0.5rem; white-space: pre-wrap;">${ver.command}\n${ver.stdout}${ver.stderr}</pre>
                </div>
            `;
        }

        element.innerHTML = html;
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
