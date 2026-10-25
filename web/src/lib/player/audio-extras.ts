import { getLoudness, getTask, getWaveform } from '../api/index';
import { TrackAnalysis } from './analysis.svelte';
import { Normalizer } from './normalizer';
import { sleep } from './prepare';

/** The one gain stage for the shared media element (book: frontend.md "音量均衡"). */
export const normalizer = new Normalizer();

/** Loudness and the waveform of the playing track. */
export const trackAnalysis = new TrackAnalysis({ getLoudness, getWaveform, getTask, sleep });
