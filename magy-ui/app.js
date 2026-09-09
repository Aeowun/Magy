const UI = {
    state: { lastSeq: 0 },

    init() {
        this.cacheElements();
        this.bindEvents();
        this.connectSSE();
        console.log("Magy v2 Chat-Only Initialized");
    },

    cacheElements() {
        this.chatFeed = document.getElementById('chat-feed');
        this.chatInput = document.getElementById('chat-input');
        this.toastRegion = document.getElementById('toast-region');
        this.relayDot = document.getElementById('relay-dot');
    },

    bindEvents() {
        document.getElementById('chat-form').addEventListener('submit', (e) => this.handleChatSubmit(e));
        this.chatInput.addEventListener('keydown', (e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                document.getElementById('chat-form').requestSubmit();
            }
        });
    },

    connectSSE() {
        const events = new EventSource('/api/events');
        events.onmessage = (e) => {
            const envelope = JSON.parse(e.data);
            if (envelope.seq <= this.state.lastSeq) return;
            this.state.lastSeq = envelope.seq;
            this.handleEvent(envelope);
        };
        events.onerror = () => {
            this.relayDot.className = 'status-dot error';
        };
        events.onopen = () => {
            this.relayDot.className = 'status-dot success';
        };
    },

    handleEvent(envelope) {
        const { type, data } = envelope;
        switch (type) {
            case 'ChatUpdate':
                this.appendChatBubble(data.role, data.content);
                break;
            case 'ThoughtReceived':
                this.appendChatBubble('magy', data);
                break;
            case 'Error':
                this.showToast(data, true);
                break;
        }
    },

    appendChatBubble(role, text) {
        const bubble = document.createElement('div');
        bubble.className = `chat-bubble ${role === 'user' ? 'user' : 'magy'}`;
        bubble.textContent = text;
        const empty = this.chatFeed.querySelector('.empty-state');
        if (empty) empty.remove();
        this.chatFeed.appendChild(bubble);
        this.chatFeed.scrollTop = this.chatFeed.scrollHeight;
        this.playPopSound();
    },

    async handleChatSubmit(e) {
        e.preventDefault();
        const text = this.chatInput.value.trim();
        if (!text) return;

        this.chatInput.value = '';
        try {
            const res = await fetch('/api/chat', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ message: text })
            });
            if (!res.ok) throw new Error("Send failed");
        } catch (e) { this.showToast(e.message, true); }
    },

    playPopSound() {
        try {
            if (!this.audioCtx) this.audioCtx = new (window.AudioContext || window.webkitAudioContext)();
            const osc = this.audioCtx.createOscillator();
            const gain = this.audioCtx.createGain();
            osc.type = 'sine';
            osc.frequency.setValueAtTime(800, this.audioCtx.currentTime);
            osc.frequency.exponentialRampToValueAtTime(100, this.audioCtx.currentTime + 0.1);
            gain.gain.setValueAtTime(0.1, this.audioCtx.currentTime);
            gain.gain.exponentialRampToValueAtTime(0.01, this.audioCtx.currentTime + 0.1);
            osc.connect(gain); gain.connect(this.audioCtx.destination);
            osc.start(); osc.stop(this.audioCtx.currentTime + 0.1);
        } catch (e) {}
    },

    showToast(msg, error = false) {
        const t = document.createElement('div');
        t.className = `toast ${error ? 'error' : ''}`;
        t.textContent = msg;
        this.toastRegion.appendChild(t);
        setTimeout(() => { t.style.opacity = '0'; setTimeout(() => t.remove(), 300); }, 4000);
    }
};

document.addEventListener('DOMContentLoaded', () => UI.init());
