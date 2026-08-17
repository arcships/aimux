import { createRouter, createWebHashHistory } from 'vue-router'
import Playground from './views/Playground.vue'
import Agent from './views/Agent.vue'
import Traces from './views/Traces.vue'
import Sessions from './views/Sessions.vue'
import Replay from './views/Replay.vue'
import CacheProbe from './views/CacheProbe.vue'
import Settings from './views/Settings.vue'

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', name: 'playground', component: Playground },
    { path: '/agent', name: 'agent', component: Agent },
    { path: '/traces', name: 'traces', component: Traces },
    { path: '/sessions', name: 'sessions', component: Sessions },
    { path: '/replay', name: 'replay', component: Replay },
    { path: '/cache-probe', name: 'cache-probe', component: CacheProbe },
    { path: '/settings', name: 'settings', component: Settings },
  ],
})
