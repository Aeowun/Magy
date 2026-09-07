/* Magy workbench: the UI speaks only to the existing HTTP/SSE contract. */
const UI = {
    state: { pendingIndex: null, lastRecord: null, lastIndex: null, activityCount: 0, verificationCount: 0, lastSeq: 0 },

    escapeHtml(value) {
        return String(value == null ? '' : value).replace(/[&<>"']/g, (character) => ({
            '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#039;'
        }[character]));
    },

    init() {
        this.cacheElements();
        this.bindEvents();
        this.connectSSE();
    },

    cacheElements() {
        const ids = [
            'load-project-btn', 'startup-view', 'startup-actions', 'init-zone', 'goal-input', 'start-init-btn', 'startup-error',
            'workspace', 'header-project-name', 'header-run-btn', 'header-cancel-btn', 'run-summary', 'agent-status', 'overview-project-name',
            'overview-goal', 'overview-task', 'overview-repo', 'overview-branch', 'overview-run-state', 'overview-evidence',
            'overview-run-btn', 'task-list', 'task-count', 'feed-container', 'activity-count', 'clear-feed-btn', 'chat-feed',
            'chat-form', 'chat-input', 'step-summary', 'verify-summary', 'refresh-github-btn', 'open-github-btn',
            'github-repo-name', 'github-branch', 'sc-status', 'auto-approve-tools', 'execution-progress', 'interaction-zone',
            'pending-tool-request', 'approve-btn', 'deny-btn', 'toast-region'
        ];
        ids.forEach((id) => { this[id.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = document.getElementById(id); });
        this.panels = Array.from(document.querySelectorAll('.panel'));
        this.railButtons = Array.from(document.querySelectorAll('.rail-button'));
    },

    bindEvents() {
        this.loadProjectBtn.addEventListener('click', () => this.loadProject());
        this.startInitBtn.addEventListener('click', () => this.initializeProject());
        this.headerRunBtn.addEventListener('click', () => this.runAgent());
        this.headerCancelBtn.addEventListener('click', () => this.cancelAgent());
        this.overviewRunBtn.addEventListener('click', () => this.runAgent());
        this.approveBtn.addEventListener('click', () => this.resolveAction(true));
        this.denyBtn.addEventListener('click', () => this.resolveAction(false));
        this.autoApproveTools.addEventListener('change', () => this.updateSettings());
        this.refreshGithubBtn.addEventListener('click', () => this.loadGithubInfo());
        this.openGithubBtn.addEventListener('click', () => { if (this.githubUrl) window.open(this.githubUrl, '_blank', 'noopener,noreferrer'); });
        this.clearFeedBtn.addEventListener('click', () => this.clearActivity());
        this.chatForm.addEventListener('submit', (event) => { event.preventDefault(); this.sendChat(); });
        this.chatInput.addEventListener('input', () => this.resizeComposer());
        this.chatInput.addEventListener('keydown', (event) => {
            if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); this.chatForm.requestSubmit(); }
        });
        this.railButtons.forEach((button) => button.addEventListener('click', () => this.showPanel(button.dataset.panel)));
        document.querySelectorAll('[data-panel-target]').forEach((button) => button.addEventListener('click', () => this.showPanel(button.dataset.panelTarget)));
        document.querySelectorAll('.quick-prompt').forEach((button) => button.addEventListener('click', () => {
            this.chatInput.value = button.dataset.prompt;
            this.resizeComposer();
            this.chatInput.focus();
        }));
        document.querySelector('.topbar-brand').addEventListener('click', (event) => { event.preventDefault(); this.showPanel('overview'); });
    },

    async secureFetch(url, options = {}) {
        const response = await fetch(url, options);
        if (!response.ok) {
            let message = `Server returned ${response.status}`;
            try { const payload = await response.json(); message = payload.message || message; } catch (_) { /* non-JSON error */ }
            throw new Error(message);
        }
        return response.json();
    },

    async loadProject() {
        this.setBusy(this.loadProjectBtn, 'Opening…');
        try {
            const data = await this.secureFetch('/api/load-project', {
                method: 'POST'
            });
            if (data.status === 'success') this.showWorkspace(data.project);
            else if (data.status === 'error' && /FileNotFound|not found|missing/i.test(data.message || '')) this.showInitZone();
            else if (data.status !== 'cancelled') this.showError('Project error', data.message);
        } catch (error) { this.showError('Could not open project', error.message); }
        this.resetBusy(this.loadProjectBtn, 'Open project');
    },

    showInitZone() {
        this.startupActions.hidden = true;
        this.initZone.hidden = false;
        this.goalInput.focus();
    },

    async initializeProject() {
        const goal = this.goalInput.value.trim();
        if (!goal) { this.showError('A goal is required', 'Describe what you want to build first.'); return; }
        this.setBusy(this.startInitBtn, 'Generating…');
        try {
            const data = await this.secureFetch('/api/initialize', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ goal }) });
            if (data.status === 'success') this.showWorkspace(data.project);
            else this.showError('Initialization failed', data.message);
        } catch (error) { this.showError('Initialization failed', error.message); }
        this.resetBusy(this.startInitBtn, 'Initialize project');
    },

    showWorkspace(project) {
        this.startupView.hidden = true;
        this.workspace.hidden = false;
        this.updateProjectUI(project);
        this.showPanel('overview');
        this.loadGithubInfo();
        this.runAgent();
    },

    updateProjectUI(project = {}) {
        const tasks = Array.isArray(project.tasks) ? project.tasks : [];
        this.headerProjectName.textContent = project.name || 'Project';
        this.overviewProjectName.textContent = project.name || 'Project';
        this.overviewGoal.textContent = project.goal || 'No project goal provided.';
        this.renderTaskList(tasks);
        this.taskCount.textContent = `${tasks.length} ${tasks.length === 1 ? 'task' : 'tasks'}`;
        const active = tasks.find((task) => String(task.status || '').toLowerCase() === 'in_progress') || tasks.find((task) => String(task.status || '').toLowerCase() !== 'done');
        this.overviewTask.textContent = active ? (active.description || 'Unnamed task') : (tasks.length ? 'All tasks complete' : 'No task selected');
    },

    renderTaskList(tasks) {
        this.taskList.replaceChildren();
        if (!tasks.length) {
            const empty = document.createElement('li');
            empty.className = 'empty-state';
            empty.textContent = 'No tasks found in this project.';
            this.taskList.appendChild(empty);
            return;
        }
        tasks.forEach((task) => {
            const item = document.createElement('li');
            const status = String(task.status || 'pending').toLowerCase();
            item.className = `task-item ${this.escapeHtml(status)}`;
            const marker = document.createElement('span');
            marker.className = 'task-marker';
            marker.setAttribute('aria-hidden', 'true');
            const body = document.createElement('div');
            body.className = 'task-body';
            const description = document.createElement('strong');
            description.textContent = task.description || 'Unnamed task';
            const statusText = document.createElement('span');
            statusText.className = 'task-status';
            statusText.textContent = status.replace('_', ' ');
            body.append(description, statusText);
            item.append(marker, body);
            this.taskList.appendChild(item);
        });
    },

    async runAgent() {
        try {
            const result = await this.secureFetch('/api/run', { method: 'POST' });
            if (result.status === 'error') this.showError('Agent error', result.message);
        } catch (error) { this.showError('Could not start agent', error.message); }
    },

    async cancelAgent() {
        this.headerCancelBtn.disabled = true;
        try {
            const result = await this.secureFetch('/api/cancel', { method: 'POST' });
            if (result.status !== 'cancellation_requested') this.showError('Could not cancel agent', result.message);
        } catch (error) { this.showError('Could not cancel agent', error.message); }
        this.headerCancelBtn.disabled = false;
    },

    async updateSettings() {
        const enabled = this.autoApproveTools.checked;
        try {
            await this.secureFetch('/api/settings', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ auto_approve_tools: enabled }) });
            this.showToast(enabled ? 'Safe tools will run automatically.' : 'Approval is required for safe tools.');
        } catch (error) {
            this.autoApproveTools.checked = !enabled;
            this.showError('Settings not saved', error.message);
        }
    },

    async resolveAction(approved) {
        const index = this.state.pendingIndex;
        this.interactionZone.hidden = true;
        if (index == null) return;
        try {
            const result = await this.secureFetch('/api/resolve', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ index, approved }) });
            if (result.status === 'success') {
                // Both decisions resolve the pending step; a denial lets the
                // runtime ask the model for a replacement action.
                this.runAgent();
            }
            else this.showError('Approval error', result.message);
        } catch (error) { this.showError('Approval error', error.message); }
    },

    connectSSE() {
        const events = new EventSource('/api/events');
        events.onmessage = (event) => {
            try { this.handleBackendEvent(JSON.parse(event.data)); } catch (_) { /* keep-alive ping */ }
        };
        events.onerror = () => { this.showToast('Live activity disconnected; reconnecting…', true); };
    },

    handleBackendEvent(event) {
        if (Number.isFinite(event.seq)) {
            if (event.seq <= this.state.lastSeq) return;
            this.state.lastSeq = event.seq;
        }
        switch (event.type) {
            case 'Snapshot': this.applySnapshot(event.data); break;
            case 'ProjectLoaded': this.updateProjectUI(event.data); break;
            case 'AgentStateChanged': this.setBackendState(event.data); break;
            case 'Step': this.appendActivity(event.data && event.data.step, event.data && event.data.index); break;
            case 'RunFinished': this.handleStop(event.data); break;
            case 'Error': this.showError('Runtime error', event.data); break;
            case 'Warning': this.appendNotice(event.data, 'warning'); break;
            case 'Chat': this.appendChat(event.data && event.data.role, event.data && event.data.content); break;
            default: break;
        }
    },

    applySnapshot(snapshot = {}) {
        if (snapshot.project) this.updateProjectUI(snapshot.project);
        const trace = snapshot.trace || {};
        const steps = Array.isArray(trace.steps) ? trace.steps : [];
        steps.forEach((step, index) => this.appendActivity(step, index));
        const state = trace.run && trace.run.state ? trace.run.state : 'idle';
        this.setBackendState(state);
        if (trace.run && trace.run.outcome) this.handleStop(trace.run.outcome);
        else if (snapshot.worker_active === false && (state === 'awaiting_approval' || state === 'stalled')) this.setRunning(false);
    },

    handleStop(outcome) {
        const kind = outcome && outcome.kind;
        this.setRunning(false);
        if (kind === 'completed') {
            this.runSummary.textContent = 'Project completed';
            this.overviewRunState.textContent = 'Completed';
        } else if (kind === 'stalled') {
            const message = (outcome.details && outcome.details.message) || 'Run stalled';
            this.runSummary.textContent = 'Run stalled';
            this.overviewRunState.textContent = 'Stalled';
            this.appendNotice(message, 'system');
            // If stalled due to a denial, show approval dialog anyway to allow override
            if (this.state.lastRecord && this.state.lastRecord.approval_status === 'denied') {
                this.showApproval();
            }
        } else if (kind === 'failed') {
            const message = (outcome.details && outcome.details.message) || 'Run failed';
            this.runSummary.textContent = 'Run failed';
            this.overviewRunState.textContent = 'Failed';
            this.appendNotice(message, 'system');
        } else if (kind === 'cancelled') {
            this.runSummary.textContent = 'Run cancelled';
            this.overviewRunState.textContent = 'Cancelled';
            this.appendNotice('Run cancelled', 'system');
        } else {
            this.runSummary.textContent = 'Run finished';
        }
    },

    setBackendState(state) {
        const labels = {
            idle: ['Idle', 'idle'],
            starting: ['Starting', 'active'],
            planning: ['Planning', 'active'],
            awaiting_model: ['Awaiting model', 'active'],
            awaiting_approval: ['Awaiting approval', 'idle'],
            executing_tool: ['Executing tool', 'active'],
            verifying: ['Verifying', 'active'],
            recovering: ['Recovering', 'active'],
            stalled: ['Stalled', 'idle'],
            completed: ['Completed', 'idle'],
            failed: ['Failed', 'error'],
            cancelled: ['Cancelled', 'idle']
        };
        const value = labels[String(state || '').toLowerCase()] || ['Idle', 'idle'];
        this.setStatus(value[0], value[1]);
        if (state === 'starting' || state === 'planning' || state === 'awaiting_model' ||
            state === 'executing_tool' || state === 'verifying' || state === 'recovering') {
            this.setRunning(true);
        } else if (state === 'awaiting_approval' || state === 'stalled') {
            this.setRunning(false);
        }
        if (state === 'awaiting_approval') this.showApproval();
        else if (state === 'stalled' && this.state.lastRecord && this.state.lastRecord.approval_status === 'denied') {
            this.showApproval();
        }
    },

    appendActivity(step = {}, index = this.state.activityCount) {
        if (document.getElementById(`step-${index}`)) return;
        if (step.action_record) {
            this.state.lastRecord = step.action_record;
            this.state.lastIndex = index;
        }
        const empty = this.feedContainer.querySelector('.empty-state');
        if (empty) empty.remove();
        const item = document.createElement('article');
        item.className = 'activity-card';
        item.id = `step-${index}`;
        const header = document.createElement('div');
        header.className = 'activity-card-header';
        const title = document.createElement('div');
        title.innerHTML = `<span class="step-number">STEP ${Number(index) + 1}</span><h2>${this.escapeHtml(step.action_record && step.action_record.request ? step.action_record.request.tool || 'Agent reasoning' : 'Agent reasoning')}</h2>`;
        const time = document.createElement('span');
        time.className = 'muted';
        time.textContent = 'Live trace';
        header.append(title, time);
        const reasoning = document.createElement('p');
        reasoning.className = 'reasoning';
        reasoning.textContent = step.model_response && step.model_response.content ? step.model_response.content : 'Magy completed a reasoning step.';
        item.append(header, reasoning);
        if (step.action_record) item.append(this.createRawDetails('Tool request and outcome', step.action_record));
        if (step.verification) {
            this.state.verificationCount += 1;
            item.append(this.createVerification(step.verification));
            this.verifySummary.textContent = `${this.state.verificationCount} ${this.state.verificationCount === 1 ? 'check' : 'checks'}`;
        }
        this.feedContainer.appendChild(item);
        this.state.activityCount += 1;
        this.activityCount.textContent = String(this.state.activityCount);
        this.activityCount.hidden = false;
        this.stepSummary.textContent = `${this.state.activityCount} ${this.state.activityCount === 1 ? 'step' : 'steps'}`;
        item.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    },

    createRawDetails(label, value) {
        const details = document.createElement('details');
        details.className = 'raw-details';
        const summary = document.createElement('summary');
        summary.textContent = label;
        const pre = document.createElement('pre');
        pre.className = 'code-block';
        pre.textContent = JSON.stringify(value, null, 2);
        details.append(summary, pre);
        return details;
    },

    createVerification(verification) {
        const block = document.createElement('div');
        const passed = Boolean(verification.passed);
        block.className = `verification ${passed ? 'passed' : 'warning'}`;
        const label = document.createElement('strong');
        label.textContent = passed ? 'Verification passed' : 'Verification warning';
        const details = document.createElement('span');
        details.textContent = verification.command ? ` · ${verification.command}` : '';
        block.append(label, details);
        return block;
    },

    appendNotice(message, kind) {
        const item = document.createElement('div');
        item.className = `notice ${kind}`;
        item.textContent = `${kind === 'warning' ? 'Warning' : 'Magy'}: ${String(message || '')}`;
        this.feedContainer.appendChild(item);
    },

    appendChat(role, content) {
        const empty = this.chatFeed.querySelector('.chat-empty');
        if (empty) empty.remove();
        const message = document.createElement('div');
        message.className = `chat-message ${role === 'user' ? 'user' : 'assistant'}`;
        message.textContent = String(content || '');
        this.chatFeed.appendChild(message);
        message.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    },

    async sendChat() {
        const message = this.chatInput.value.trim();
        if (!message) return;
        this.chatInput.value = '';
        this.resizeComposer();
        try { await this.secureFetch('/api/chat', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ message }) }); }
        catch (error) { this.showError('Chat unavailable', error.message); }
    },

    async loadGithubInfo() {
        try {
            const data = await this.secureFetch('/api/github-info');
            if (data.status !== 'success') throw new Error(data.message || 'Repository unavailable');
            const info = data.github || {};
            this.githubUrl = info.github_url;
            const name = info.github_url ? info.github_url.replace(/^https:\/\/github\.com\//, '') : (info.is_git_repository ? 'Local Git repository' : 'Not a Git repository');
            const branch = info.branch ? `${info.branch} · ${info.changed_files || 0} changed` : 'No branch detected';
            this.githubRepoName.textContent = name;
            this.githubBranch.textContent = branch;
            this.overviewRepo.textContent = name;
            this.overviewBranch.textContent = branch;
            this.scStatus.classList.toggle('success', Boolean(info.github_url || info.is_git_repository));
            this.openGithubBtn.classList.toggle('hidden', !info.github_url);
        } catch (error) {
            this.githubRepoName.textContent = 'Repository unavailable';
            this.githubBranch.textContent = error.message;
            this.overviewRepo.textContent = 'Unavailable';
            this.overviewBranch.textContent = 'Could not inspect Git metadata';
        }
    },

    showApproval() {
        if (!this.state.lastRecord || !this.state.lastRecord.request) {
            this.showError('Approval unavailable', 'The pending action details were not received.');
            return;
        }
        this.pendingToolRequest.textContent = JSON.stringify(this.state.lastRecord.request, null, 2);
        this.state.pendingIndex = this.state.lastIndex;
        this.interactionZone.hidden = false;
        this.approveBtn.focus();
    },

    showPanel(panelName) {
        const valid = this.panels.some((panel) => panel.id === `${panelName}-panel`);
        if (!valid) return;
        this.panels.forEach((panel) => {
            const active = panel.id === `${panelName}-panel`;
            panel.hidden = !active;
            panel.classList.toggle('active-panel', active);
        });
        this.railButtons.forEach((button) => {
            const active = button.dataset.panel === panelName;
            button.classList.toggle('active', active);
            if (active) button.setAttribute('aria-current', 'page'); else button.removeAttribute('aria-current');
        });
    },

    clearActivity() {
        this.feedContainer.replaceChildren();
        const empty = document.createElement('div');
        empty.className = 'empty-state';
        empty.innerHTML = '<span class="empty-glyph" aria-hidden="true">✦</span><h2>Activity cleared</h2><p>New actions will appear here.</p>';
        this.feedContainer.appendChild(empty);
        this.state.activityCount = 0;
        this.activityCount.hidden = true;
        this.stepSummary.textContent = '0 steps';
    },

    setRunning(running) {
        this.executionProgress.hidden = !running;
        this.headerRunBtn.disabled = running;
        this.headerCancelBtn.hidden = !running;
        this.headerCancelBtn.disabled = false;
        this.overviewRunBtn.disabled = running;
        if (running) { this.setStatus('Executing', 'active'); this.runSummary.textContent = 'Working through the active task…'; this.overviewRunState.textContent = 'Running'; }
    },

    setStatus(label, status) {
        this.agentStatus.textContent = String(label || 'Idle');
        this.agentStatus.dataset.status = status || (/error|failed/i.test(label) ? 'error' : /execut/i.test(label) ? 'active' : 'idle');
    },

    resizeComposer() {
        this.chatInput.style.height = 'auto';
        this.chatInput.style.height = `${Math.min(this.chatInput.scrollHeight, 150)}px`;
    },

    setBusy(button, label) { button.disabled = true; button.dataset.originalLabel = button.textContent; button.textContent = label; },
    resetBusy(button, label) { button.disabled = false; button.textContent = label; },

    showError(title, message) {
        this.startupError.hidden = false;
        this.startupError.textContent = `${title}: ${message || 'Unknown error'}`;
        this.showToast(`${title}: ${message || 'Unknown error'}`, true);
        if (!this.workspace.hidden) this.setStatus('Error', 'error');
    },

    showToast(message, error = false) {
        const toast = document.createElement('div');
        toast.className = `toast ${error ? 'error' : ''}`;
        toast.textContent = message;
        this.toastRegion.appendChild(toast);
        window.setTimeout(() => toast.remove(), 4500);
    }
};

document.addEventListener('DOMContentLoaded', () => UI.init());
