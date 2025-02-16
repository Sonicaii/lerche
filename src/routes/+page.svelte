<script>
    import { onMount, onDestroy } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { getCurrentWindow } from "@tauri-apps/api/window";

    const handleMouseDown = async (event) => {
        if (event.button === 0) {
            await getCurrentWindow().startDragging();
        }
    };

    let captureCanvas;
    let ctx;
    let animationFrameId;
    let resizeObserver;
    let imageData;
    let fps = 'None';

    let req_prev = Math.floor(Date.now() / 1000);
    let req_current = 0;
    let req_count = 0;

    function updateCanvasSize() {
        if (captureCanvas && window) {
            captureCanvas.width = window.innerWidth;
            captureCanvas.height = window.innerHeight;
        }
    }

    async function updateFrame() {
        try {
            const [bytes, width, height, new_fps] = await invoke('get_frame_data');

            if (!ctx) return;

            fps = new_fps.toString() || fps;  // Ensure FPS updates

            req_current += 1;
            let now = Math.floor(Date.now() / 1000)
            if (req_prev < now) {
                req_count = req_current;
                req_prev = now;
                req_current = 0;
            }

            // Create or update ImageData if dimensions changed
            if (!imageData || imageData.width !== width || imageData.height !== height) {
                imageData = new ImageData(width, height);
            }

            // Update pixel data
            const typedBytes = new Uint8Array(bytes);
            imageData.data.set(typedBytes);

            // Scale to fit window while maintaining aspect ratio
            const scale = Math.min(
                captureCanvas.width / width,
                captureCanvas.height / height
            );

            // Clear canvas
            ctx.clearRect(0, 0, captureCanvas.width, captureCanvas.height);

            // Draw scaled image data
            const scaledWidth = width * scale;
            const scaledHeight = height * scale;
            const x = (captureCanvas.width - scaledWidth) / 2;
            const y = (captureCanvas.height - scaledHeight) / 2;

            // Create temporary canvas for scaling
            const tempCanvas = new OffscreenCanvas(width, height);
            const tempCtx = tempCanvas.getContext('2d');
            tempCtx.putImageData(imageData, 0, 0);

            // Draw scaled image
            ctx.imageSmoothingEnabled = false;
            ctx.drawImage(tempCanvas, x, y, scaledWidth, scaledHeight);
        } catch (error) {
            console.error('Frame update error:', error);
        }

        // Schedule next frame
        animationFrameId = requestAnimationFrame(updateFrame);
    }

    onMount( () => {
        ctx = captureCanvas.getContext('2d', {
            alpha: false,
            desynchronized: true
        });
        ctx.imageSmoothingEnabled = false;
        updateCanvasSize();

        // Start the animation loop
        animationFrameId = requestAnimationFrame(updateFrame);

        // Handle window resizing
        resizeObserver = new ResizeObserver(updateCanvasSize);
        resizeObserver.observe(document.body);
    });

    onDestroy(() => {
        if (animationFrameId) {
            cancelAnimationFrame(animationFrameId);
        }
        if (resizeObserver) resizeObserver.disconnect();
    });
</script>
<div class="root" on:mousedown={handleMouseDown} role="presentation">
    <div class="canvas-container">
            <div class="fps">FPS: {fps}<br>Requests Per Second: {req_count}</div>
        <canvas
                bind:this={captureCanvas}
                class="capture-canvas"
        ></canvas>
        <div class="canvas-overlay"></div>
    </div>
</div>

<style>
    .fps {
        color: white;
        position: absolute;
        top: 50%;
        left: 50%;
        transform: translate(-50%, -50%);
        z-index: 1;
    }

    .root {
        position: fixed;
        top: 2px;
        left: 2px;
        width: calc(100% - 4px);
        height: calc(100% - 4px);
        -webkit-app-region: drag;
        app-region: drag;
        border-radius: 50px;
        overflow: hidden;
        border: white 1px solid;
    }

    .canvas-container {
        position: relative;
        width: 100%;
        height: 100%;
        border-radius: 50px;
        overflow: hidden;
    }

    .capture-canvas {
        position: absolute;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
    }

    .canvas-overlay {
        position: absolute;
        backdrop-filter: blur(10px);
        display: block;
        width: 100%;
        height: 100%;
        border-radius: 50px;
    }

    :global(body) {
        margin: 0;
        padding: 0;
        overflow: hidden;
    }
</style>