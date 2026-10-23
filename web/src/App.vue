<script setup lang="ts">
import SiteNav from './components/SiteNav.vue'
import HeroSection from './components/HeroSection.vue'
import DemoSection from './components/DemoSection.vue'
import ChannelsSection from './components/ChannelsSection.vue'
import ArchitectureSection from './components/ArchitectureSection.vue'
import CompareSection from './components/CompareSection.vue'
import InstallSection from './components/InstallSection.vue'
import ClosingSection from './components/ClosingSection.vue'
import SiteFooter from './components/SiteFooter.vue'
</script>

<template>
  <a class="skip-link" href="#main">跳到主要内容</a>

  <SiteNav />

  <main id="main" tabindex="-1">
    <HeroSection />
    <DemoSection />
    <ChannelsSection />
    <ArchitectureSection />
    <CompareSection />
    <InstallSection />
    <ClosingSection />
  </main>

  <SiteFooter />
</template>
