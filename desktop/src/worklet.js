class PCMProcessor extends AudioWorkletProcessor {
    process(inputs, outputs, parameters) {
        const input = inputs[0];
        if (input.length > 0) {
            const channel = input[0]; // Float32Array
            // Convert Float32 (-1.0 to 1.0) to Int16 (-32768 to 32767)
            const pcm16 = new Int16Array(channel.length);
            for (let i = 0; i < channel.length; i++) {
                let s = Math.max(-1, Math.min(1, channel[i]));
                pcm16[i] = s < 0 ? s * 0x8000 : s * 0x7FFF;
            }
            // Send the raw PCM buffer to the main thread
            this.port.postMessage(pcm16.buffer, [pcm16.buffer]);
        }
        return true;
    }
}

registerProcessor('pcm-processor', PCMProcessor);
