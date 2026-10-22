<script setup lang="ts">
import SiteNav from './components/SiteNav.vue'
import HeroSection from './components/HeroSection.vue'
import FeaturesBento from './components/FeaturesBento.vue'
import ScreenshotsSection from './components/ScreenshotsSection.vue'
import CompareSection from './components/CompareSection.vue'
import InstallSection from './components/InstallSection.vue'
import FinalCta from './components/FinalCta.vue'
import SiteFooter from './components/SiteFooter.vue'
</script>

<template>
  <SiteNav />

  <main id="top">
    <HeroSection />
    <FeaturesBento />
    <ScreenshotsSection />
    <CompareSection />
    <InstallSection />
    <FinalCta />
  </main>

  <SiteFooter />
</template>
