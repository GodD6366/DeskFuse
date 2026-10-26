<script setup lang="ts">
import SiteNav from './components/SiteNav.vue'
import DeskTour from './components/DeskTour.vue'
import PrivacySection from './components/PrivacySection.vue'
import CompareSection from './components/CompareSection.vue'
import InstallSection from './components/InstallSection.vue'
import HelpSection from './components/HelpSection.vue'
import SiteFooter from './components/SiteFooter.vue'
</script>

<template>
  <a class="skip-link" href="#main">跳到主要内容</a>
  <SiteNav />
  <main id="main" tabindex="-1">
    <DeskTour />
    <PrivacySection />
    <CompareSection />
    <InstallSection />
    <HelpSection />
  </main>
  <SiteFooter />
</template>
