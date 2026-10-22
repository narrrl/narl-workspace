import '@fontsource-variable/jetbrains-mono'
import '@fontsource-variable/noto-sans'
import { mount } from 'svelte'
import App from './App.svelte'
import './app.css'

export default mount(App, { target: document.getElementById('app')! })

// The worker that lets the app be installed and opened offline; see sw.ts.
// Not in development, where it would keep Vite's modules.
if (import.meta.env.PROD && 'serviceWorker' in navigator) void navigator.serviceWorker.register('/sw.js')
