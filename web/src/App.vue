<script setup lang="ts">
import SiteNav from './components/SiteNav.vue'
import HomeHero from './components/home/HomeHero.vue'
import CapabilitySection from './components/home/CapabilitySection.vue'
import AdvantageSection from './components/home/AdvantageSection.vue'
import MobileSection from './components/home/MobileSection.vue'
import CompareSection from './components/CompareSection.vue'
import DownloadSection from './components/home/DownloadSection.vue'
import HelpSection from './components/HelpSection.vue'
import FinalCta from './components/home/FinalCta.vue'
import SiteFooter from './components/SiteFooter.vue'
</script>

<template>
  <a class="skip-link" href="#main">跳到主要内容</a>
  <SiteNav />
  <main id="main" tabindex="-1">
    <HomeHero />
    <CapabilitySection />
    <AdvantageSection />
    <MobileSection />
    <CompareSection />
    <DownloadSection />
    <HelpSection />
    <FinalCta />
  </main>
  <SiteFooter />
</template>
