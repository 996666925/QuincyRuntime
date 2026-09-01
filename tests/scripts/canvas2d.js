"use strict";

const canvas = document.createElement("canvas");
canvas.width = 640;
canvas.height = 480;
const ctx = canvas.getContext("2d");

ctx.fillStyle = "#18243a";
ctx.fillRect(0, 0, canvas.width, canvas.height);
ctx.save();
ctx.globalAlpha = 0.9;
ctx.fillStyle = "#e84a5f";
ctx.fillRect(80, 60, 480, 300);
ctx.restore();

ctx.beginPath();
ctx.moveTo(320, 90);
ctx.lineTo(170, 330);
ctx.lineTo(470, 330);
ctx.lineTo(320, 90);
ctx.strokeStyle = "#ffffff";
ctx.lineWidth = 6;
ctx.stroke();

"Canvas 2D triangle rendered";
