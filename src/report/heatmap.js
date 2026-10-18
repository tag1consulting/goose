(function () {
    // The report's response time heatmap. The data comes from the JSON
    // element Goose writes before this script; request names reach the page
    // only as the text of the select's options, never as markup.
    var holder = document.getElementById('graph-latency-heatmap');
    var source = document.getElementById('graph-latency-heatmap-data');
    if (!holder || !source || typeof echarts === 'undefined') {
        return;
    }
    var data = JSON.parse(source.textContent);
    if (!data || !data.series || data.series.length === 0) {
        return;
    }
    var select = document.getElementById('graph-latency-heatmap-series');
    var views = document.getElementById('graph-latency-heatmap-views');
    var buttons = views.querySelectorAll('button');
    var note = document.getElementById('graph-latency-heatmap-width');
    var legendMin = document.getElementById('graph-latency-heatmap-min');
    var legendMax = document.getElementById('graph-latency-heatmap-max');
    var chart = echarts.init(holder);
    var view = 'adjusted';

    data.series.forEach(function (series, index) {
        var option = document.createElement('option');
        option.value = String(index);
        option.textContent = series.name;
        select.appendChild(option);
    });

    // The dashboard's colour ramp: one hue, light for the sparsest cells
    // and dark for the busiest. `level` runs from 0 to 1000.
    function color(level) {
        var t = level / 1000;
        var saturation = 80 + 5 * t;
        var lightness = 64 - 42 * t;
        return 'hsl(222, ' + saturation.toFixed(1) + '%, ' + lightness.toFixed(1) + '%)';
    }
    document.getElementById('graph-latency-heatmap-ramp').style.background =
        'linear-gradient(to right, ' + color(0) + ', ' + color(1000) + ')';

    // A bucket's span on the logarithmic axis: from its lowest time to past
    // its highest. Times under a millisecond start at 0.5 ms.
    function span(bucket) {
        var row = data.buckets[bucket];
        return [row[1] === 0 ? 0.5 : row[1], row[2] + 1];
    }

    function pad(number) {
        return (number < 10 ? '0' : '') + number;
    }

    // Times are shifted to the Goose host's zone and drawn as UTC.
    function clock(ms) {
        var date = new Date(ms);
        return date.getUTCFullYear() + '-' + pad(date.getUTCMonth() + 1) + '-' +
            pad(date.getUTCDate()) + ' ' + pad(date.getUTCHours()) + ':' +
            pad(date.getUTCMinutes()) + ':' + pad(date.getUTCSeconds());
    }

    function requests(count) {
        return count.toLocaleString() + (count === 1 ? ' request' : ' requests');
    }

    function bucketText(bucket) {
        var row = data.buckets[bucket];
        if (row[2] === 0) {
            return 'under 1 ms';
        }
        if (row[1] === row[2]) {
            return row[0].toLocaleString() + ' ms';
        }
        return row[0].toLocaleString() + ' ms bucket (' + row[1].toLocaleString() +
            ' to ' + row[2].toLocaleString() + ' ms)';
    }

    function tooltip(params) {
        var value = params.value;
        if (params.seriesType === 'custom') {
            return clock(value[0]) + ' to ' + clock(value[1]) + '<br>' +
                bucketText(value[6]) + ': ' + requests(value[5]) + '<br>' +
                'p50 / p95 / p99: ' + value[7] + ' / ' + value[8] + ' / ' + value[9] + ' ms';
        }
        return clock(value[0]) + '<br>' + params.seriesName + ': ' +
            value[2].toLocaleString() + ' ms';
    }

    function draw() {
        var series = data.series[Number(select.value)];
        var adjustable = series.adjusted !== null;
        views.hidden = !adjustable;
        var adjusted = adjustable && view === 'adjusted';
        for (var b = 0; b < buttons.length; b++) {
            var pressed = (buttons[b].getAttribute('data-view') === 'adjusted') === adjusted;
            buttons[b].setAttribute('aria-pressed', pressed ? 'true' : 'false');
            buttons[b].className = pressed ? 'active' : '';
        }
        var shown = adjusted ? series.adjusted : series.measured;
        var widthMs = series.width * 1000;
        var first = data.start + data.offset + data.origin * 1000;
        var cells = [];
        var lines = [[], [], []];
        var low = Infinity;
        var high = 0;
        var end = first;
        var previous = null;
        shown.columns.forEach(function (column) {
            var from = first + column[0] * widthMs;
            var to = from + widthMs;
            end = Math.max(end, to);
            // A column with no requests breaks the lines.
            if (previous !== null && column[0] > previous + 1) {
                for (var gap = 0; gap < 3; gap++) {
                    lines[gap].push([from - widthMs / 2, null, null]);
                }
            }
            previous = column[0];
            for (var p = 0; p < 3; p++) {
                // The value axis is logarithmic, so a percentile under a
                // millisecond is drawn at 0.5 ms, as its bucket starts.
                lines[p].push([(from + to) / 2, Math.max(column[1 + p], 0.5), column[1 + p]]);
            }
            for (var i = 4; i + 2 < column.length; i += 3) {
                var bucketSpan = span(column[i]);
                low = Math.min(low, bucketSpan[0]);
                high = Math.max(high, bucketSpan[1]);
                cells.push([from, to, bucketSpan[0], bucketSpan[1], column[i + 2],
                    column[i + 1], column[i], column[1], column[2], column[3]]);
            }
        });
        if (cells.length === 0) {
            low = 0.5;
            high = 1000;
        }
        note.textContent = 'Each column is ' +
            (series.width === 1 ? '1 second' : series.width + ' seconds') +
            '. Color shows how many requests took that long; the lines are each column\'s p50, p95 and p99.';
        legendMin.textContent = requests(shown.min);
        legendMax.textContent = requests(shown.max);
        var line = function (name, color, values) {
            return {
                name: name,
                type: 'line',
                symbol: 'none',
                connectNulls: false,
                lineStyle: { color: color, width: 1.5 },
                itemStyle: { color: color },
                encode: { x: 0, y: 1 },
                data: values
            };
        };
        chart.setOption({
            animation: false,
            useUTC: true,
            tooltip: { trigger: 'item', formatter: tooltip },
            legend: { data: ['p50', 'p95', 'p99'] },
            grid: { left: 80, right: 40, top: 40, bottom: 50 },
            xAxis: { type: 'time', min: first, max: Math.max(end, first + widthMs) },
            yAxis: {
                name: 'Response time [ms]',
                nameLocation: 'center',
                nameRotate: 90,
                nameGap: 55,
                type: 'log',
                logBase: 10,
                min: low,
                max: high
            },
            series: [
                {
                    type: 'custom',
                    // Drawn in one pass, not over several frames, so a
                    // printed report has every cell.
                    progressive: 0,
                    renderItem: function (params, api) {
                        var topLeft = api.coord([api.value(0), api.value(3)]);
                        var bottomRight = api.coord([api.value(1), api.value(2)]);
                        return {
                            type: 'rect',
                            shape: {
                                x: topLeft[0],
                                y: topLeft[1],
                                width: Math.max(bottomRight[0] - topLeft[0], 1),
                                height: Math.max(bottomRight[1] - topLeft[1], 1)
                            },
                            style: { fill: color(api.value(4)) }
                        };
                    },
                    encode: { x: [0, 1], y: [2, 3] },
                    data: cells
                },
                line('p50', '#2c664f', lines[0]),
                line('p95', '#d4730b', lines[1]),
                line('p99', '#b33f3f', lines[2])
            ]
        }, true);
    }

    select.addEventListener('change', draw);
    for (var b = 0; b < buttons.length; b++) {
        buttons[b].addEventListener('click', function (event) {
            view = event.currentTarget.getAttribute('data-view');
            draw();
        });
    }
    draw();
})();
