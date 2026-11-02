"""Transcribes a real recording with the backend this machine would use (CUDA, MLX or CPU).

Slow (loads a Whisper model), so it only runs when PF_SPEECH_WAV points to a 16 kHz mono WAV file:
    PF_SPEECH_WAV=speech.wav PF_SPEECH_EXPECT=button,blue PF_TEST_MODEL=base pytest tests/test_speech.py -s
"""
import os
import time
import wave

import numpy as np
import pytest

import promptflow as pf

WAV = os.environ.get("PF_SPEECH_WAV")


def read_wav(path):
    with wave.open(path) as w:
        assert w.getframerate() == pf.SAMPLE_RATE and w.getnchannels() == 1 and w.getsampwidth() == 2
        return np.frombuffer(w.readframes(w.getnframes()), np.int16).astype(np.float32) / 32768


@pytest.mark.skipif(not WAV, reason="set PF_SPEECH_WAV to run")
def test_transcribe():
    pf.MODEL = os.environ.get("PF_TEST_MODEL", pf.MODEL)
    t = time.perf_counter()
    model, backend = pf.load_model()
    print(f"\nbackend: {backend}, model: {pf.MODEL}, loaded in {time.perf_counter() - t:.1f}s")

    audio = read_wav(WAV)
    t = time.perf_counter()
    text, lang = pf.transcribe(model, audio, pf.INITIAL_PROMPT)
    print(f"[{time.perf_counter() - t:.2f}s, {lang}] {text}")
    for word in os.environ.get("PF_SPEECH_EXPECT", "").split(","):
        if word:
            assert word.lower() in text.lower()

    silence, _ = pf.transcribe(model, np.zeros(pf.SAMPLE_RATE * 2, np.float32), pf.INITIAL_PROMPT)
    assert silence == ""
