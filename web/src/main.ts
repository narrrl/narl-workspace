import '@fontsource-variable/jetbrains-mono'
import '@fontsource-variable/noto-sans'
import { mount } from 'svelte'
import App from './App.svelte'
import './app.css'

export default mount(App, { target: document.getElementById('app')! })
