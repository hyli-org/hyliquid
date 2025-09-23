import { createApp } from "vue";
import { createRouter, createWebHistory } from "vue-router";
import "./style.css";
import App from "./App.vue";
import Trade from "./trade/Trade.vue";

const routes = [
    { path: "/", redirect: "/trade" },
    { path: "/trade", component: Trade },
];

export const router = createRouter({
    history: createWebHistory(),
    routes,
});

createApp(App).use(router).mount("#app");
