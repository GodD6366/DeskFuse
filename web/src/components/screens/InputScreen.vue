<script setup lang="ts">
import DeskStage from '../DeskStage.vue'

defineProps<{ active?: boolean }>()
</script>

<template>
  <div class="sc-host">
    <div class="sc">
      <div class="sc__copy">
        <h2>一套键鼠，划过屏幕边缘就换电脑</h2>
        <p>
          光标移出这块屏幕，就出现在相邻那台电脑上，键盘输入也跟过去。屏幕按你桌上的真实位置摆放，不用记方向。
        </p>
        <ul class="sc__list">
          <li>已配对、在线的电脑自动加入排布</li>
          <li>一个快捷键随时把控制收回本机</li>
          <li>Logitech 鼠标的侧键可以设成快捷键，几台 Mac 共用一套设置（Beta）</li>
        </ul>
      </div>
      <div class="sc__demo">
        <DeskStage />
      </div>
    </div>
  </div>
</template>
